//! Module dedicated to the Maildir backend configuration.
//!
//! This module contains the configuration specific to the Maildir
//! backend.

use shellexpand_utils::shellexpand_path;
use std::{hash::Hash, path::PathBuf};

/// The Maildir backend configuration.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
#[cfg_attr(
    feature = "derive",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "kebab-case")
)]
pub struct MaildirConfig {
    /// The Maildir root directory.
    ///
    /// The path should point to the root level of the Maildir
    /// directory (the one containing the `cur`, `new` and `tmp`
    /// folders). Path is shell-expanded, which means environment
    /// variables and tilde `~` are replaced by their values.
    pub root_dir: PathBuf,
}

#[cfg(feature = "account-sync")]
impl crate::sync::hash::SyncHash for MaildirConfig {
    fn sync_hash(&self, state: &mut std::hash::DefaultHasher) {
        shellexpand_path(&self.root_dir).hash(state);
    }
}
