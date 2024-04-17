use async_trait::async_trait;
use utf7_imap::encode_utf7_imap as encode_utf7;

use super::DeleteFolder;
use crate::{debug, folder::error::Error, imap::ImapContextSync, info, AnyResult};

#[derive(Debug)]
pub struct DeleteImapFolder {
    ctx: ImapContextSync,
}

impl DeleteImapFolder {
    pub fn new(ctx: &ImapContextSync) -> Self {
        Self { ctx: ctx.clone() }
    }

    pub fn new_boxed(ctx: &ImapContextSync) -> Box<dyn DeleteFolder> {
        Box::new(Self::new(ctx))
    }

    pub fn some_new_boxed(ctx: &ImapContextSync) -> Option<Box<dyn DeleteFolder>> {
        Some(Self::new_boxed(ctx))
    }
}

#[async_trait]
impl DeleteFolder for DeleteImapFolder {
    async fn delete_folder(&self, folder: &str) -> AnyResult<()> {
        info!("deleting imap folder {folder}");

        let mut ctx = self.ctx.lock().await;
        let config = &ctx.account_config;

        let folder = config.get_folder_alias(folder);
        let folder_encoded = encode_utf7(folder.clone());
        debug!("utf7 encoded folder: {folder_encoded}");

        ctx.exec(
            |session| session.delete(&folder_encoded),
            |err| Error::DeleteFolderImapError(err, folder.clone()),
        )
        .await?;

        Ok(())
    }
}
