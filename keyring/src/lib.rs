#[doc(inline)]
pub use keyring_native as native;
use log::debug;
use once_cell::sync::OnceCell;
use std::{ops::Deref, result};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot get keyring entry {1}")]
    GetEntryError(#[source] native::Error, String),
    #[error("cannot get keyring entry secret for key {1}")]
    GetSecretError(#[source] native::Error, String),
    #[error("cannot find keyring entry secret for key {1}")]
    FindSecretError(#[source] native::Error, String),
    #[error("cannot set keyring entry secret for key {1}")]
    SetSecretError(#[source] native::Error, String),
    #[error("cannot delete keyring entry secret for key {1}")]
    DeleteSecretError(#[source] native::Error, String),
}

pub type Result<T> = result::Result<T, Error>;

static SERVICE_NAME: OnceCell<&str> = OnceCell::with_value("keyring-lib");

/// Define a new global keyring service name.
///
/// The service name will be used every time a new native entry is
/// created.
pub fn set_global_service_name(next: &'static str) {
    if let Err(prev) = SERVICE_NAME.set(next) {
        debug!("global keyring service name {prev} replaced by {next}");
    } else {
        debug!("global keyring service name set to {next}");
    }
}

/// Alias for the keyring entry key.
pub type Key = String;

/// Keyring entry wrapper.
///
/// This struct is a simple wrapper around [`native::Entry`] that
/// holds a keyring entry key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry(Key);

impl Entry {
    /// Create a new native keyring entry instance.
    fn new_native_entry(&self) -> Result<native::Entry> {
        // a service name is always present, so unwrap() is safe here
        let service_name = SERVICE_NAME.get().unwrap();
        native::Entry::new(service_name, &self)
            .map_err(|err| Error::GetEntryError(err, self.0.clone()))
    }

    /// Create a new keyring entry with a keyring entry key.
    pub fn new(key: impl ToString) -> Self {
        Self(key.to_string())
    }

    /// Get the inner key of the keyring entry.
    pub fn get_key(&self) -> &str {
        debug!("getting keyring entry key: {}", self.0);
        self.as_str()
    }

    /// Get the secret of the keyring entry.
    pub fn get_secret(&self) -> Result<String> {
        debug!("getting keyring entry secret for key {:?}", self.0);
        self.new_native_entry()?
            .get_password()
            .map_err(|err| Error::GetSecretError(err, self.0.clone()))
    }

    /// Find the secret of the keyring entry. Return None in case the
    /// secret is not found.
    pub fn find_secret(&self) -> Result<Option<String>> {
        debug!("finding keyring entry secret for key {:?}", self.0);
        match self.new_native_entry()?.get_password() {
            Err(native::Error::NoEntry) => Ok(None),
            Err(err) => Err(Error::FindSecretError(err, self.0.clone())),
            Ok(secret) => Ok(Some(secret)),
        }
    }

    /// (Re)set the secret of the keyring entry.
    pub fn set_secret(&self, secret: impl AsRef<str>) -> Result<()> {
        debug!("setting keyring entry secret for key {:?}", self.0);
        self.new_native_entry()?
            .set_password(secret.as_ref())
            .map_err(|err| Error::SetSecretError(err, self.0.clone()))
    }

    /// Delete the secret of the keyring entry.
    pub fn delete_secret(&self) -> Result<()> {
        debug!("deleting keyring entry secret for key {:?}", self.0);
        self.new_native_entry()?
            .delete_password()
            .map_err(|err| Error::DeleteSecretError(err, self.0.clone()))
    }
}

impl Deref for Entry {
    type Target = Key;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<String> for Entry {
    fn from(key: String) -> Self {
        Self::new(key)
    }
}

impl From<&String> for Entry {
    fn from(key: &String) -> Self {
        Self::new(key)
    }
}

impl From<&str> for Entry {
    fn from(key: &str) -> Self {
        Self::new(key)
    }
}

impl Into<String> for Entry {
    fn into(self) -> String {
        self.0
    }
}

impl ToString for Entry {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}
