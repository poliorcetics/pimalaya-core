use async_trait::async_trait;
use chrono::Duration;
use log::{debug, info, trace};
use thiserror::Error;

use crate::{
    folder::FolderKind,
    notmuch::NotmuchContextSync,
    search_query::{filter::SearchEmailsQueryFilter, SearchEmailsQuery},
    Result,
};

use super::{Envelopes, ListEnvelopes, ListEnvelopesOptions};

#[derive(Error, Debug)]
pub enum Error {
    #[error("cannot list notmuch envelopes from {0}: page {1} out of bounds")]
    GetEnvelopesOutOfBoundsError(String, usize),
    #[error("cannot list notmuch envelopes from {0}: invalid query {1}")]
    SearchMessagesInvalidQuery(#[source] notmuch::Error, String, String),
}

#[derive(Clone)]
pub struct ListNotmuchEnvelopes {
    ctx: NotmuchContextSync,
}

impl ListNotmuchEnvelopes {
    pub fn new(ctx: &NotmuchContextSync) -> Self {
        Self { ctx: ctx.clone() }
    }

    pub fn new_boxed(ctx: &NotmuchContextSync) -> Box<dyn ListEnvelopes> {
        Box::new(Self::new(ctx))
    }

    pub fn some_new_boxed(ctx: &NotmuchContextSync) -> Option<Box<dyn ListEnvelopes>> {
        Some(Self::new_boxed(ctx))
    }
}

#[async_trait]
impl ListEnvelopes for ListNotmuchEnvelopes {
    async fn list_envelopes(&self, folder: &str, opts: ListEnvelopesOptions) -> Result<Envelopes> {
        info!("listing notmuch envelopes from folder {folder}");

        let ctx = self.ctx.lock().await;
        let config = &ctx.account_config;
        let db = ctx.open_db()?;

        let mut final_query = if FolderKind::matches_inbox(folder) {
            String::from("folder:\"\"")
        } else {
            let folder = config.get_folder_alias(folder.as_ref());
            format!("folder:{folder:?}")
        };

        if let Some(query) = opts.query.as_ref() {
            let query = query.to_notmuch_search_query();
            if !query.is_empty() {
                final_query.push_str(" and ");
                final_query.push_str(&query);
            }
        }

        let query_builder = db.create_query(&final_query)?;

        let msgs = query_builder.search_messages().map_err(|err| {
            Error::SearchMessagesInvalidQuery(err, folder.to_owned(), final_query.clone())
        })?;

        let mut envelopes = Envelopes::from_notmuch_msgs(msgs);

        let envelopes_len = envelopes.len();
        debug!("found {envelopes_len} notmuch envelopes matching query {final_query}");
        trace!("{envelopes:#?}");

        let page_begin = opts.page * opts.page_size;

        if page_begin > envelopes.len() {
            return Err(Error::GetEnvelopesOutOfBoundsError(
                folder.to_owned(),
                page_begin + 1,
            ))?;
        }

        let page_end = envelopes.len().min(if opts.page_size == 0 {
            envelopes.len()
        } else {
            page_begin + opts.page_size
        });

        opts.sort_envelopes(&mut envelopes);
        *envelopes = envelopes[page_begin..page_end].into();

        db.close()?;

        Ok(envelopes)
    }
}

impl SearchEmailsQuery {
    pub fn to_notmuch_search_query(&self) -> String {
        self.filters
            .as_ref()
            .map(|f| f.to_notmuch_search_query())
            .unwrap_or_default()
    }
}

impl SearchEmailsQueryFilter {
    pub fn to_notmuch_search_query(&self) -> String {
        let mut query = String::new();

        match self {
            SearchEmailsQueryFilter::And(left, right) => {
                query.push_str("(");
                query.push_str(&left.to_notmuch_search_query());
                query.push_str(") and (");
                query.push_str(&right.to_notmuch_search_query());
                query.push(')');
            }
            SearchEmailsQueryFilter::Or(left, right) => {
                query.push_str("(");
                query.push_str(&left.to_notmuch_search_query());
                query.push_str(") or (");
                query.push_str(&right.to_notmuch_search_query());
                query.push(')');
            }
            SearchEmailsQueryFilter::Not(right) => {
                query.push_str("not (");
                query.push_str(&right.to_notmuch_search_query());
                query.push_str(")");
            }
            SearchEmailsQueryFilter::Date(date) => {
                query.push_str("date:");
                query.push_str(&date.to_string());
            }
            SearchEmailsQueryFilter::BeforeDate(date) => {
                // notmuch dates are inclusive, so we substract one
                // day from the before date filter.
                let date = *date - Duration::days(1);
                query.push_str("date:..");
                query.push_str(&date.to_string());
            }
            SearchEmailsQueryFilter::AfterDate(date) => {
                // notmuch dates are inclusive, so we add one day to
                // the after date filter.
                let date = *date + Duration::days(1);
                query.push_str("date:");
                query.push_str(&date.to_string());
                query.push_str("..");
            }
            SearchEmailsQueryFilter::From(pattern) => {
                query.push_str("from:");
                query.push_str(pattern);
            }

            SearchEmailsQueryFilter::To(pattern) => {
                query.push_str("to:");
                query.push_str(pattern);
            }
            SearchEmailsQueryFilter::Subject(pattern) => {
                query.push_str("subject:");
                query.push_str(pattern);
            }
            SearchEmailsQueryFilter::Body(pattern) => {
                query.push_str("body:");
                query.push_str(pattern);
            }
            SearchEmailsQueryFilter::Keyword(pattern) => {
                query.push_str("keyword:");
                query.push_str(pattern);
            }
        };

        query
    }
}
