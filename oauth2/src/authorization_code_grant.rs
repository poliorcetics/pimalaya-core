//! Authorization Grant Code flow helper, as defined in the
//! [RFC6749](https://datatracker.ietf.org/doc/html/rfc6749#section-1.3.1)

use oauth2::{
    basic::{BasicClient, BasicErrorResponseType},
    url::{self, Url},
    AuthorizationCode, CsrfToken, PkceCodeChallenge, PkceCodeVerifier, RequestTokenError, Scope,
    StandardErrorResponse, TokenResponse,
};
use std::io;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

use crate::Result;

#[derive(Error, Debug)]
pub enum Error {
    #[error("cannot build auth url")]
    BuildAuthUrlError(#[source] oauth2::url::ParseError),
    #[error("cannot build token url")]
    BuildTokenUrlError(#[source] oauth2::url::ParseError),
    #[error("cannot build revocation url")]
    BuildRevocationUrlError(#[source] oauth2::url::ParseError),
    #[error("cannot build redirect url")]
    BuildRedirectUrlError(#[source] oauth2::url::ParseError),
    #[error("cannot bind redirect server")]
    BindRedirectServerError(String, u16, #[source] io::Error),
    #[error("cannot accept redirect server connections")]
    AcceptRedirectServerError(#[source] io::Error),
    #[error("invalid state {0}: expected {1}")]
    InvalidStateError(String, String),
    #[error("missing redirect url from {0}")]
    MissingRedirectUrlError(String),
    #[error("cannot parse redirect url {1}")]
    ParseRedirectUrlError(#[source] url::ParseError, String),
    #[error("cannot find code from redirect url {0}")]
    FindCodeInRedirectUrlError(Url),
    #[error("cannot find state from redirect url {0}")]
    FindStateInRedirectUrlError(Url),
    #[error("cannot exchange code for an access token and a refresh token")]
    ExchangeCodeError(
        RequestTokenError<
            oauth2::reqwest::Error<reqwest::Error>,
            StandardErrorResponse<BasicErrorResponseType>,
        >,
    ),
}

/// OAuth 2.0 Authorization Code Grant flow builder.
///
/// The first step (once the builder is configured) is to build a
/// [`crate::Client`].
///
/// The second step is to get the redirect URL by calling
/// [`AuthorizationCodeGrant::get_redirect_url`].
///
/// The last step is to spawn a redirect server and wait for the user
/// to click on the redirect URL in order to extract the access token
/// and the refresh token by calling
/// [`AuthorizationCodeGrant::wait_for_redirection`].
#[derive(Debug)]
pub struct AuthorizationCodeGrant {
    pub scopes: Vec<Scope>,
    pub pkce: Option<(PkceCodeChallenge, PkceCodeVerifier)>,
    pub redirect_host: String,
    pub redirect_port: u16,
}

impl AuthorizationCodeGrant {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_scope<T>(mut self, scope: T) -> Self
    where
        T: ToString,
    {
        self.scopes.push(Scope::new(scope.to_string()));
        self
    }

    pub fn with_pkce(mut self) -> Self {
        self.pkce = Some(PkceCodeChallenge::new_random_sha256());
        self
    }

    pub fn with_redirect_host<T>(mut self, host: T) -> Self
    where
        T: ToString,
    {
        self.redirect_host = host.to_string();
        self
    }

    pub fn with_redirect_port<T>(mut self, port: T) -> Self
    where
        T: Into<u16>,
    {
        self.redirect_port = port.into();
        self
    }

    /// Generate the redirect URL used to complete the OAuth 2.0
    /// Authorization Code Grant flow.
    pub fn get_redirect_url(&self, client: &BasicClient) -> (Url, CsrfToken) {
        let mut redirect = client
            .authorize_url(CsrfToken::new_random)
            .add_scopes(self.scopes.clone());

        if let Some((pkce_challenge, _)) = &self.pkce {
            redirect = redirect.set_pkce_challenge(pkce_challenge.clone());
        }

        redirect.url()
    }

    /// Wait for the user to click on the redirect URL generated by
    /// [`AuthorizationCodeGrant::get_redirect_url`], then exchange
    /// the received code with an access token and maybe a refresh
    /// token.
    pub async fn wait_for_redirection(
        self,
        client: &BasicClient,
        csrf_state: CsrfToken,
    ) -> Result<(String, Option<String>)> {
        let host = self.redirect_host;
        let port = self.redirect_port;

        // listen for one single connection
        let (mut stream, _) = TcpListener::bind((host.clone(), port))
            .await
            .map_err(|err| Error::BindRedirectServerError(host, port, err))?
            .accept()
            .await
            .map_err(Error::AcceptRedirectServerError)?;

        // extract the code from the url
        let code = {
            let mut reader = BufReader::new(&mut stream);

            let mut request_line = String::new();
            reader.read_line(&mut request_line).await?;

            let redirect_url = request_line
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| Error::MissingRedirectUrlError(request_line.clone()))?;
            let redirect_url = format!("http://localhost{redirect_url}");
            let redirect_url = Url::parse(&redirect_url)
                .map_err(|err| Error::ParseRedirectUrlError(err, redirect_url.clone()))?;

            let (_, state) = redirect_url
                .query_pairs()
                .find(|(key, _)| key == "state")
                .ok_or_else(|| Error::FindStateInRedirectUrlError(redirect_url.clone()))?;
            let state = CsrfToken::new(state.into_owned());

            if state.secret() != csrf_state.secret() {
                return Ok(Err(Error::InvalidStateError(
                    state.secret().to_owned(),
                    csrf_state.secret().to_owned(),
                ))?);
            }

            let (_, code) = redirect_url
                .query_pairs()
                .find(|(key, _)| key == "code")
                .ok_or_else(|| Error::FindCodeInRedirectUrlError(redirect_url.clone()))?;

            AuthorizationCode::new(code.into_owned())
        };

        // write a basic http response in plain text
        let res = "Authentication successful!";
        let res = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            res.len(),
            res
        );
        stream.write_all(res.as_bytes()).await?;

        // exchange the code for an access token and a refresh token
        let mut res = client.exchange_code(code);

        if let Some((_, pkce_verifier)) = self.pkce {
            res = res.set_pkce_verifier(pkce_verifier);
        }

        let res = res
            .request_async(oauth2::reqwest::async_http_client)
            .await
            .map_err(Error::ExchangeCodeError)?;

        let access_token = res.access_token().secret().to_owned();
        let refresh_token = res.refresh_token().map(|t| t.secret().clone());

        Ok((access_token, refresh_token))
    }
}

impl Default for AuthorizationCodeGrant {
    fn default() -> Self {
        Self {
            scopes: Vec::new(),
            pkce: None,
            redirect_host: String::from("localhost"),
            redirect_port: 9999,
        }
    }
}
