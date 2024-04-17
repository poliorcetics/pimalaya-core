use async_trait::async_trait;
use utf7_imap::encode_utf7_imap as encode_utf7;

use super::CopyMessages;
use crate::{debug, email::error::Error, envelope::Id, imap::ImapContextSync, info, AnyResult};

#[derive(Clone, Debug)]
pub struct CopyImapMessages {
    ctx: ImapContextSync,
}

impl CopyImapMessages {
    pub fn new(ctx: &ImapContextSync) -> Self {
        Self { ctx: ctx.clone() }
    }

    pub fn new_boxed(ctx: &ImapContextSync) -> Box<dyn CopyMessages> {
        Box::new(Self::new(ctx))
    }

    pub fn some_new_boxed(ctx: &ImapContextSync) -> Option<Box<dyn CopyMessages>> {
        Some(Self::new_boxed(ctx))
    }
}

#[async_trait]
impl CopyMessages for CopyImapMessages {
    async fn copy_messages(&self, from_folder: &str, to_folder: &str, id: &Id) -> AnyResult<()> {
        info!("copying imap messages {id} from folder {from_folder} to folder {to_folder}");

        let mut ctx = self.ctx.lock().await;
        let config = &ctx.account_config;

        let from_folder = config.get_folder_alias(from_folder);
        let from_folder_encoded = encode_utf7(from_folder.clone());
        debug!("utf7 encoded from folder: {from_folder_encoded}");

        let to_folder = config.get_folder_alias(to_folder);
        let to_folder_encoded = encode_utf7(to_folder.clone());
        debug!("utf7 encoded to folder: {to_folder_encoded}");

        ctx.exec(
            |session| session.select(&from_folder_encoded),
            |err| Error::SelectFolderImapError(err, from_folder.clone()),
        )
        .await?;

        ctx.exec(
            |session| session.uid_copy(id.join(","), &to_folder_encoded),
            |err| {
                Error::CopyMessagesImapError(
                    err,
                    from_folder.clone(),
                    to_folder.clone(),
                    id.clone(),
                )
            },
        )
        .await?;

        Ok(())
    }
}
