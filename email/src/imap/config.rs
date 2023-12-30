//! Module dedicated to the IMAP backend configuration.
//!
//! This module contains the implementation of the IMAP backend and
//! all associated structures related to it.

use imap::ConnectionMode;
use serde::{de, Deserialize, Deserializer, Serialize};
use std::{fmt, marker::PhantomData, result};
use thiserror::Error;

use crate::{
    account::config::{oauth2::OAuth2Config, passwd::PasswdConfig},
    Result,
};

/// Errors related to the IMAP backend configuration.
#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot get imap password from global keyring")]
    GetPasswdError(#[source] secret::Error),
    #[error("cannot get imap password: password is empty")]
    GetPasswdEmptyError,
}

/// The IMAP backend configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ImapConfig {
    /// The IMAP server host name.
    pub host: String,

    /// The IMAP server host port.
    pub port: u16,

    /// The IMAP encryption protocol to use.
    ///
    /// Supported encryption: SSL/TLS, STARTTLS or none.
    #[serde(default, deserialize_with = "some_bool_or_kind")]
    pub encryption: Option<ImapEncryptionKind>,

    /// The IMAP server login.
    ///
    /// Usually, the login is either the email address or its left
    /// part (before @).
    pub login: String,

    /// The IMAP server authentication configuration.
    ///
    /// Authentication can be done using password or OAuth 2.0.
    /// See [ImapAuthConfig].
    #[serde(flatten)]
    pub auth: ImapAuthConfig,

    /// The IMAP notify command.
    ///
    /// Defines the command used to notify the user when a new email is available.
    /// Defaults to `notify-send "📫 <sender>" "<subject>"`.
    pub watch: Option<ImapWatchConfig>,
}

impl ImapConfig {
    /// Return `true` if TLS or StartTLS is enabled.
    pub fn is_encryption_enabled(&self) -> bool {
        match self.encryption.as_ref() {
            None => true,
            Some(ImapEncryptionKind::Tls) => true,
            Some(ImapEncryptionKind::StartTls) => true,
            _ => false,
        }
    }

    /// Return `true` if StartTLS is enabled.
    pub fn is_start_tls_encryption_enabled(&self) -> bool {
        match self.encryption.as_ref() {
            Some(ImapEncryptionKind::StartTls) => true,
            _ => false,
        }
    }

    /// Return `true` if encryption is disabled.
    pub fn is_encryption_disabled(&self) -> bool {
        match self.encryption.as_ref() {
            Some(ImapEncryptionKind::None) => true,
            _ => false,
        }
    }

    /// Builds authentication credentials.
    ///
    /// Authentication credentials can be either a password or an
    /// OAuth 2.0 access token.
    pub async fn build_credentials(&self) -> Result<String> {
        self.auth.build_credentials().await
    }

    /// Find the IMAP watch timeout.
    pub fn find_watch_timeout(&self) -> Option<u64> {
        self.watch.as_ref().and_then(|c| c.find_timeout())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImapEncryptionKind {
    #[default]
    #[serde(alias = "ssl")]
    Tls,
    #[serde(alias = "starttls")]
    StartTls,
    None,
}

impl fmt::Display for ImapEncryptionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tls => write!(f, "SSL/TLS"),
            Self::StartTls => write!(f, "StartTLS"),
            Self::None => write!(f, "None"),
        }
    }
}

impl From<bool> for ImapEncryptionKind {
    fn from(value: bool) -> Self {
        if value {
            Self::Tls
        } else {
            Self::None
        }
    }
}

impl Into<ConnectionMode> for ImapEncryptionKind {
    fn into(self) -> ConnectionMode {
        match self {
            Self::Tls => ConnectionMode::Tls,
            Self::StartTls => ConnectionMode::StartTls,
            Self::None => ConnectionMode::Plaintext,
        }
    }
}

/// The IMAP authentication configuration.
///
/// Authentication can be done using password or OAuth 2.0.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "auth")]
pub enum ImapAuthConfig {
    /// The password configuration.
    #[serde(alias = "password")]
    Passwd(PasswdConfig),

    /// The OAuth 2.0 configuration.
    #[serde(alias = "oauth2")]
    OAuth2(OAuth2Config),
}

impl Default for ImapAuthConfig {
    fn default() -> Self {
        Self::Passwd(Default::default())
    }
}

impl ImapAuthConfig {
    /// Builds authentication credentials.
    ///
    /// Authentication credentials can be either a password or an
    /// OAuth 2.0 access token.
    pub async fn build_credentials(&self) -> Result<String> {
        match self {
            ImapAuthConfig::Passwd(passwd) => {
                let passwd = passwd.get().await.map_err(Error::GetPasswdError)?;
                let passwd = passwd.lines().next().ok_or(Error::GetPasswdEmptyError)?;
                Ok(passwd.to_owned())
            }
            ImapAuthConfig::OAuth2(oauth2) => Ok(oauth2.access_token().await?),
        }
    }

    pub fn replace_undefined_keyring_entries(&mut self, name: impl AsRef<str>) {
        let name = name.as_ref();

        match self {
            Self::Passwd(secret) => {
                secret.set_keyring_entry_if_undefined(format!("{name}-imap-passwd"));
            }
            Self::OAuth2(config) => {
                config
                    .client_secret
                    .set_keyring_entry_if_undefined(format!("{name}-imap-oauth2-client-secret"));
                config
                    .access_token
                    .set_keyring_entry_if_undefined(format!("{name}-imap-oauth2-access-token"));
                config
                    .refresh_token
                    .set_keyring_entry_if_undefined(format!("{name}-imap-oauth2-refresh-token"));
            }
        }
    }
}

/// The IMAP watch options (IDLE).
///
/// Options dedicated to the IMAP IDLE mode, which is used to watch
/// changes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ImapWatchConfig {
    /// The IMAP watch timeout.
    ///
    /// Timeout used to refresh the IDLE command in
    /// background. Defaults to 29 min as defined in the RFC.
    timeout: Option<u64>,
}

impl ImapWatchConfig {
    /// Find the IMAP watch timeout.
    pub fn find_timeout(&self) -> Option<u64> {
        self.timeout
    }
}

fn some_bool_or_kind<'de, D>(
    deserializer: D,
) -> result::Result<Option<ImapEncryptionKind>, D::Error>
where
    D: Deserializer<'de>,
{
    struct SomeBoolOrKind(PhantomData<fn() -> Option<ImapEncryptionKind>>);

    impl<'de> de::Visitor<'de> for SomeBoolOrKind {
        type Value = Option<ImapEncryptionKind>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("some or none")
        }

        fn visit_some<D>(self, deserializer: D) -> result::Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            struct BoolOrKind(PhantomData<fn() -> ImapEncryptionKind>);

            impl<'de> de::Visitor<'de> for BoolOrKind {
                type Value = ImapEncryptionKind;

                fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                    formatter.write_str("boolean or string")
                }

                fn visit_bool<E>(self, v: bool) -> result::Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    Ok(v.into())
                }

                fn visit_str<E>(self, v: &str) -> result::Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    Deserialize::deserialize(de::value::StrDeserializer::new(v))
                }
            }

            deserializer
                .deserialize_any(BoolOrKind(PhantomData))
                .map(Option::Some)
        }
    }

    deserializer.deserialize_option(SomeBoolOrKind(PhantomData))
}
