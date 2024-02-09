//! Module dedicated to email synchronization.
//!
//! The core concept of this module is the [`EmailSyncPatchManager`],
//! which allows you to synchronize remote emails using a local
//! Maildir backend.

mod cache;
mod hunk;
pub mod patch;
mod report;
mod runner;
pub mod worker;

use futures::Future;
use log::debug;
use std::{collections::HashMap, fmt, path::PathBuf, pin::Pin, sync::Arc};
use thiserror::Error;

use crate::{
    account::config::AccountConfig,
    backend::{BackendBuilder, BackendContextBuilder},
    envelope::Envelope,
    maildir::{config::MaildirConfig, MaildirContextBuilder},
    sync::SyncDestination,
    Result,
};

use self::patch::build_patch;
#[doc(inline)]
pub use self::{
    cache::EmailSyncCache,
    hunk::{EmailSyncCacheHunk, EmailSyncHunk},
    patch::{EmailSyncCachePatch, EmailSyncPatch, EmailSyncPatchManager},
    report::EmailSyncReport,
    runner::EmailSyncRunner,
};

/// Errors related to email synchronization.
#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot get email sync cache directory")]
    GetCacheDirectoryError,

    // TODO: sort me
    #[error("cannot find email by internal id {0}")]
    FindEmailError(String),
}

pub type EmailSyncEventHandler =
    dyn Fn(EmailSyncEvent) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> + Send + Sync;

/// The backend synchronization progress event.
///
/// Represents all the events that can be triggered during the backend
/// synchronization process.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum EmailSyncEvent {
    ListedLeftEnvelopes(String, usize),
    ListedLeftCachedEnvelopes(String, usize),
    ListedRightEnvelopes(String, usize),
    ListedRightCachedEnvelopes(String, usize),
    ListedAllEnvelopes(String),
    ProcessedEmailHunk(EmailSyncHunk),
}

impl EmailSyncEvent {
    pub async fn emit(&self, handler: &Option<Arc<EmailSyncEventHandler>>) {
        debug!("emitting email sync event {self:?}");

        if let Some(handler) = handler.as_ref() {
            if let Err(err) = handler(self.clone()).await {
                debug!("error while emitting email sync event: {err:?}");
            }
        }
    }
}

impl fmt::Display for EmailSyncEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use EmailSyncEvent::*;

        match self {
            ListedLeftEnvelopes(folder, n) => {
                write!(f, "Listed {n} left envelopes in {folder}")
            }
            ListedLeftCachedEnvelopes(folder, n) => {
                write!(f, "Listed {n} left cached envelopes in {folder}")
            }
            ListedRightEnvelopes(folder, n) => {
                write!(f, "Listed {n} right envelopes in {folder}")
            }
            ListedRightCachedEnvelopes(folder, n) => {
                write!(f, "Listed {n} right cached envelopes in {folder}")
            }
            ListedAllEnvelopes(folder) => {
                write!(f, "Listed all envelopes in {folder}")
            }
            ProcessedEmailHunk(hunk) => {
                write!(f, "{hunk}")
            }
        }
    }
}

#[derive(Clone)]
pub struct EmailSyncBuilder<L, R>
where
    L: BackendContextBuilder + 'static,
    R: BackendContextBuilder + 'static,
{
    id: String,
    left_builder: BackendBuilder<L>,
    right_builder: BackendBuilder<R>,
    handler: Option<Arc<EmailSyncEventHandler>>,
    cache_dir: Option<PathBuf>,
}

impl<L, R> EmailSyncBuilder<L, R>
where
    L: BackendContextBuilder + 'static,
    R: BackendContextBuilder + 'static,
{
    pub fn new(left_builder: BackendBuilder<L>, right_builder: BackendBuilder<R>) -> Self {
        let id = left_builder.account_config.name.clone() + &right_builder.account_config.name;
        let id = format!("{:x}", md5::compute(id));

        Self {
            id,
            left_builder,
            right_builder,
            handler: None,
            cache_dir: None,
        }
    }

    pub fn set_some_handler<F: Future<Output = Result<()>> + Send + 'static>(
        &mut self,
        handler: Option<impl Fn(EmailSyncEvent) -> F + Send + Sync + 'static>,
    ) {
        self.handler = match handler {
            Some(handler) => Some(Arc::new(move |evt| Box::pin(handler(evt)))),
            None => None,
        };
    }

    pub fn set_handler<F: Future<Output = Result<()>> + Send + 'static>(
        &mut self,
        handler: impl Fn(EmailSyncEvent) -> F + Send + Sync + 'static,
    ) {
        self.set_some_handler(Some(handler));
    }

    pub fn with_some_handler<F: Future<Output = Result<()>> + Send + 'static>(
        mut self,
        handler: Option<impl Fn(EmailSyncEvent) -> F + Send + Sync + 'static>,
    ) -> Self {
        self.set_some_handler(handler);
        self
    }

    pub fn with_some_atomic_handler_ref(
        mut self,
        handler: Option<Arc<EmailSyncEventHandler>>,
    ) -> Self {
        self.handler = handler;
        self
    }

    pub fn with_handler<F: Future<Output = Result<()>> + Send + 'static>(
        mut self,
        handler: impl Fn(EmailSyncEvent) -> F + Send + Sync + 'static,
    ) -> Self {
        self.set_handler(handler);
        self
    }

    pub fn set_some_cache_dir(&mut self, dir: Option<impl Into<PathBuf>>) {
        self.cache_dir = dir.map(Into::into);
    }

    pub fn set_cache_dir(&mut self, dir: impl Into<PathBuf>) {
        self.set_some_cache_dir(Some(dir));
    }

    pub fn with_some_cache_dir(mut self, dir: Option<impl Into<PathBuf>>) -> Self {
        self.set_some_cache_dir(dir);
        self
    }

    pub fn with_cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.set_cache_dir(dir);
        self
    }

    pub fn find_default_cache_dir(&self) -> Option<PathBuf> {
        dirs::cache_dir().map(|dir| {
            dir.join("pimalaya")
                .join("email")
                .join("sync")
                .join(&self.id)
        })
    }

    pub fn find_cache_dir(&self) -> Option<PathBuf> {
        self.cache_dir
            .as_ref()
            .cloned()
            .or_else(|| self.find_default_cache_dir())
    }

    pub fn get_cache_dir(&self) -> Result<PathBuf> {
        self.find_cache_dir()
            .ok_or(Error::GetCacheDirectoryError.into())
    }

    pub async fn sync(self, folder: impl ToString) -> Result<EmailSyncReport> {
        let cache_dir = self.get_cache_dir()?;
        let left_config = self.left_builder.account_config.clone();
        let right_config = self.left_builder.account_config.clone();

        let folder_clone = folder.to_string();
        let handler_clone = self.handler.clone();
        let root_dir = cache_dir.join(&left_config.name);
        let ctx = MaildirContextBuilder::new(Arc::new(MaildirConfig { root_dir }));
        let left_cached_builder = BackendBuilder::new(left_config.clone(), ctx);
        let left_cached_builder_clone = left_cached_builder.clone();
        let left_envelopes_cached = tokio::spawn(async move {
            let envelopes: HashMap<String, Envelope> = HashMap::from_iter(
                left_cached_builder_clone
                    .build()
                    .await?
                    .list_envelopes(&folder_clone, 0, 0)
                    .await?
                    .into_iter()
                    .map(|e| (e.message_id.clone(), e)),
            );

            EmailSyncEvent::ListedLeftCachedEnvelopes(folder_clone, envelopes.len())
                .emit(&handler_clone)
                .await;

            Result::Ok(envelopes)
        });

        let folder_clone = folder.to_string();
        let handler_clone = self.handler.clone();
        let left_builder_clone = self.left_builder.clone();
        let left_envelopes = tokio::spawn(async move {
            let envelopes: HashMap<String, Envelope> = HashMap::from_iter(
                left_builder_clone
                    .build()
                    .await?
                    .list_envelopes(&folder_clone, 0, 0)
                    .await?
                    .into_iter()
                    .map(|e| (e.message_id.clone(), e)),
            );

            EmailSyncEvent::ListedLeftEnvelopes(folder_clone, envelopes.len())
                .emit(&handler_clone)
                .await;

            Result::Ok(envelopes)
        });

        let folder_clone = folder.to_string();
        let handler_clone = self.handler.clone();
        let root_dir = cache_dir.join(&right_config.name);
        let ctx = MaildirContextBuilder::new(Arc::new(MaildirConfig { root_dir }));
        let right_cached_builder = BackendBuilder::new(right_config.clone(), ctx);
        let right_cached_builder_clone = right_cached_builder.clone();
        let right_envelopes_cached = tokio::spawn(async move {
            let envelopes: HashMap<String, Envelope> = HashMap::from_iter(
                right_cached_builder_clone
                    .build()
                    .await?
                    .list_envelopes(&folder_clone, 0, 0)
                    .await?
                    .into_iter()
                    .map(|e| (e.message_id.clone(), e)),
            );

            EmailSyncEvent::ListedRightCachedEnvelopes(folder_clone, envelopes.len())
                .emit(&handler_clone)
                .await;

            Result::Ok(envelopes)
        });

        let folder_clone = folder.to_string();
        let handler_clone = self.handler.clone();
        let right_builder_clone = self.right_builder.clone();
        let right_envelopes = tokio::spawn(async move {
            let envelopes: HashMap<String, Envelope> = HashMap::from_iter(
                right_builder_clone
                    .build()
                    .await?
                    .list_envelopes(&folder_clone, 0, 0)
                    .await?
                    .into_iter()
                    .map(|e| (e.message_id.clone(), e)),
            );

            EmailSyncEvent::ListedRightEnvelopes(folder_clone, envelopes.len())
                .emit(&handler_clone)
                .await;

            Result::Ok(envelopes)
        });

        let (left_envelopes_cached, left_envelopes, right_envelopes_cached, right_envelopes) = tokio::try_join!(
            left_envelopes_cached,
            left_envelopes,
            right_envelopes_cached,
            right_envelopes,
        )?;

        EmailSyncEvent::ListedAllEnvelopes(folder.to_string())
            .emit(&self.handler)
            .await;

        let patch = build_patch(
            folder,
            left_envelopes_cached?,
            left_envelopes?,
            right_envelopes_cached?,
            right_envelopes?,
        );

        let report = worker::process_patch(
            self.left_builder.clone(),
            left_cached_builder.clone(),
            self.right_builder.clone(),
            right_cached_builder.clone(),
            self.handler,
            patch.into_iter().collect(),
            8,
        )
        .await;

        Ok(report)
    }
}
