//! Module dedicated to the SMTP sender.
//!
//! This module contains the implementation of the SMTP sender and all
//! associated structures related to it.

pub mod config;

use async_trait::async_trait;
use log::{debug, warn};
use mail_parser::{Address, HeaderName, HeaderValue, Message, MessageParser};
use mail_send::{smtp::message as smtp, SmtpClientBuilder};
use std::collections::HashSet;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use crate::{account::AccountConfig, sender::Sender, Result};

#[doc(inline)]
pub use self::config::{SmtpAuthConfig, SmtpConfig};

/// Errors related to the SMTP sender.
#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot send email without a sender")]
    SendEmailMissingSenderError,
    #[error("cannot send email without a recipient")]
    SendEmailMissingRecipientError,
    #[error("cannot send email")]
    SendEmailError(#[source] mail_send::Error),
    #[error("cannot connect to smtp server using tcp")]
    ConnectTcpError(#[source] mail_send::Error),
    #[error("cannot connect to smtp server using tls")]
    ConnectTlsError(#[source] mail_send::Error),
}

enum SmtpClient {
    Tcp(mail_send::SmtpClient<TcpStream>),
    Tls(mail_send::SmtpClient<TlsStream<TcpStream>>),
}

impl SmtpClient {
    pub async fn send<'a>(&mut self, msg: impl smtp::IntoMessage<'a>) -> mail_send::Result<()> {
        match self {
            Self::Tcp(client) => client.send(msg).await,
            Self::Tls(client) => client.send(msg).await,
        }
    }
}

/// The SMTP sender.
pub struct Smtp {
    account_config: AccountConfig,
    smtp_config: SmtpConfig,
    client_builder: SmtpClientBuilder<String>,
    client: SmtpClient,
}

impl Smtp {
    /// Creates a new SMTP sender from configurations.
    pub async fn new(account_config: AccountConfig, smtp_config: SmtpConfig) -> Result<Self> {
        let mut client_builder = SmtpClientBuilder::new(smtp_config.host.clone(), smtp_config.port)
            .credentials(smtp_config.credentials().await?)
            .implicit_tls(!smtp_config.starttls());

        if smtp_config.insecure() {
            client_builder = client_builder.allow_invalid_certs();
        }

        let (client_builder, client) = Self::build_client(&smtp_config, client_builder).await?;

        Ok(Self {
            account_config,
            smtp_config,
            client_builder,
            client,
        })
    }

    async fn build_client(
        smtp_config: &SmtpConfig,
        mut client_builder: SmtpClientBuilder<String>,
    ) -> Result<(SmtpClientBuilder<String>, SmtpClient)> {
        match (&smtp_config.auth, smtp_config.ssl()) {
            (SmtpAuthConfig::Passwd(_), false) => {
                let client = Self::build_tcp_client(&client_builder).await?;
                Ok((client_builder, client))
            }
            (SmtpAuthConfig::Passwd(_), true) => {
                let client = Self::build_tls_client(&client_builder).await?;
                Ok((client_builder, client))
            }
            (SmtpAuthConfig::OAuth2(oauth2_config), false) => {
                match Ok(Self::build_tcp_client(&client_builder).await?) {
                    Ok(client) => Ok((client_builder, client)),
                    Err(Error::ConnectTcpError(mail_send::Error::AuthenticationFailed(_))) => {
                        oauth2_config.refresh_access_token().await?;
                        client_builder =
                            client_builder.credentials(smtp_config.credentials().await?);
                        let client = Self::build_tcp_client(&client_builder).await?;
                        Ok((client_builder, client))
                    }
                    Err(err) => Ok(Err(err)?),
                }
            }
            (SmtpAuthConfig::OAuth2(oauth2_config), true) => {
                match Ok(Self::build_tls_client(&client_builder).await?) {
                    Ok(client) => Ok((client_builder, client)),
                    Err(Error::ConnectTlsError(mail_send::Error::AuthenticationFailed(_))) => {
                        oauth2_config.refresh_access_token().await?;
                        client_builder =
                            client_builder.credentials(smtp_config.credentials().await?);
                        let client = Self::build_tls_client(&client_builder).await?;
                        Ok((client_builder, client))
                    }
                    Err(err) => Ok(Err(err)?),
                }
            }
        }
    }

    async fn build_tcp_client(client_builder: &SmtpClientBuilder<String>) -> Result<SmtpClient> {
        match client_builder.connect_plain().await {
            Ok(client) => Ok(SmtpClient::Tcp(client)),
            Err(err) => Ok(Err(Error::ConnectTcpError(err))?),
        }
    }

    async fn build_tls_client(client_builder: &SmtpClientBuilder<String>) -> Result<SmtpClient> {
        match client_builder.connect().await {
            Ok(client) => Ok(SmtpClient::Tls(client)),
            Err(err) => Ok(Err(Error::ConnectTlsError(err))?),
        }
    }

    async fn send(&mut self, msg: &[u8]) -> Result<()> {
        let buffer: Vec<u8>;

        let mut msg = MessageParser::new().parse(msg).unwrap_or_else(|| {
            warn!("cannot parse raw message");
            Default::default()
        });

        if let Some(cmd) = self.account_config.email_hooks.pre_send.as_ref() {
            match cmd.run_with(msg.raw_message()).await {
                Ok(res) => {
                    buffer = res.into();
                    msg = MessageParser::new().parse(&buffer).unwrap_or_else(|| {
                        warn!("cannot parse raw message");
                        Default::default()
                    });
                }
                Err(err) => {
                    warn!("cannot execute pre-send hook: {err}");
                    debug!("cannot execute pre-send hook {cmd:?}: {err:?}");
                }
            }
        };

        match &self.smtp_config.auth {
            SmtpAuthConfig::Passwd(_) => {
                self.client
                    .send(into_smtp_msg(msg)?)
                    .await
                    .map_err(Error::SendEmailError)?;
                Ok(())
            }
            SmtpAuthConfig::OAuth2(oauth2_config) => {
                match self.client.send(into_smtp_msg(msg.clone())?).await {
                    Ok(()) => Ok(()),
                    Err(mail_send::Error::AuthenticationFailed(_)) => {
                        oauth2_config.refresh_access_token().await?;
                        self.client_builder = self
                            .client_builder
                            .clone()
                            .credentials(self.smtp_config.credentials().await?);
                        self.client = if self.smtp_config.ssl() {
                            Self::build_tls_client(&self.client_builder).await
                        } else {
                            Self::build_tcp_client(&self.client_builder).await
                        }?;

                        self.client
                            .send(into_smtp_msg(msg)?)
                            .await
                            .map_err(Error::SendEmailError)?;
                        Ok(())
                    }
                    Err(err) => Ok(Err(Error::SendEmailError(err))?),
                }
            }
        }
    }
}

#[async_trait]
impl Sender for Smtp {
    async fn send(&mut self, msg: &[u8]) -> Result<()> {
        Ok(self.send(msg).await?)
    }
}

/// Transforms a [`mail_parser::Message`] into a [`mail_send::smtp::message::Message`].
///
/// This function returns an error if no sender or no recipient is
/// found in the original message.
fn into_smtp_msg<'a>(msg: Message<'a>) -> Result<smtp::Message<'a>> {
    let mut mail_from = None;
    let mut rcpt_to = HashSet::new();

    for header in msg.headers() {
        let key = &header.name;
        let val = header.value();

        match key {
            HeaderName::From => match val {
                HeaderValue::Address(Address::List(addrs)) => {
                    if let Some(addr) = addrs.first() {
                        if let Some(ref email) = addr.address {
                            mail_from = email.to_string().into();
                        }
                    }
                }
                HeaderValue::Address(Address::Group(groups)) => {
                    if let Some(group) = groups.first() {
                        if let Some(ref addr) = group.addresses.first() {
                            if let Some(ref email) = addr.address {
                                mail_from = email.to_string().into();
                            }
                        }
                    }
                }
                _ => (),
            },
            HeaderName::To | HeaderName::Cc | HeaderName::Bcc => match val {
                HeaderValue::Address(Address::List(addrs)) => {
                    if let Some(addr) = addrs.first() {
                        if let Some(ref email) = addr.address {
                            rcpt_to.insert(email.to_string());
                        }
                    }
                }
                HeaderValue::Address(Address::Group(groups)) => {
                    if let Some(group) = groups.first() {
                        if let Some(ref addr) = group.addresses.first() {
                            if let Some(ref email) = addr.address {
                                {
                                    rcpt_to.insert(email.to_string());
                                }
                            }
                        }
                    }
                }
                _ => (),
            },
            _ => (),
        };
    }

    if rcpt_to.is_empty() {
        return Ok(Err(Error::SendEmailMissingRecipientError)?);
    }

    let msg = smtp::Message {
        mail_from: mail_from.ok_or(Error::SendEmailMissingSenderError)?.into(),
        rcpt_to: rcpt_to
            .into_iter()
            .map(|email| smtp::Address {
                email: email.into(),
                parameters: Default::default(),
            })
            .collect(),
        body: msg.raw_message.into(),
    };

    Ok(msg)
}
