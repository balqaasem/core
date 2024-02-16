//! Module dedicated to backend management.
//!
//! The core concept of this module is the [`Backend`] trait, which is
//! an abstraction over emails manipulation.
//!
//! Then you have the [`BackendConfig`] which represents the
//! backend-specific configuration, mostly used by the
//! [AccountConfiguration](crate::account::config::AccountConfig).

pub mod macros {
    pub use email_macros::BackendContext;
}

use async_trait::async_trait;
use paste::paste;
#[allow(unused)]
use std::sync::Arc;
use thiserror::Error;

use crate::{
    account::config::AccountConfig,
    envelope::{get::GetEnvelope, watch::WatchEnvelopes, Envelope, Envelopes},
    envelope::{list::ListEnvelopes, Id, SingleId},
    flag::{add::AddFlags, remove::RemoveFlags, set::SetFlags, Flag, Flags},
    folder::{
        add::AddFolder, delete::DeleteFolder, expunge::ExpungeFolder, list::ListFolders,
        purge::PurgeFolder, Folders,
    },
    message::{
        add::AddMessage, copy::CopyMessages, delete::DeleteMessages, get::GetMessages,
        peek::PeekMessages, r#move::MoveMessages, send::SendMessage, Messages,
    },
    Result,
};

/// Errors related to backend.
#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot add folder: feature not available")]
    AddFolderNotAvailableError,
    #[error("cannot list folders: feature not available")]
    ListFoldersNotAvailableError,
    #[error("cannot expunge folder: feature not available")]
    ExpungeFolderNotAvailableError,
    #[error("cannot purge folder: feature not available")]
    PurgeFolderNotAvailableError,
    #[error("cannot delete folder: feature not available")]
    DeleteFolderNotAvailableError,
    #[error("cannot list envelopes: feature not available")]
    ListEnvelopesNotAvailableError,
    #[error("cannot watch for envelopes changes: feature not available")]
    WatchEnvelopesNotAvailableError,
    #[error("cannot get envelope: feature not available")]
    GetEnvelopeNotAvailableError,
    #[error("cannot add flag(s): feature not available")]
    AddFlagsNotAvailableError,
    #[error("cannot set flag(s): feature not available")]
    SetFlagsNotAvailableError,
    #[error("cannot remove flag(s): feature not available")]
    RemoveFlagsNotAvailableError,
    #[error("cannot add message: feature not available")]
    AddMessageNotAvailableError,
    #[error("cannot add message with flags: feature not available")]
    AddMessageWithFlagsNotAvailableError,
    #[error("cannot send message: feature not available")]
    SendMessageNotAvailableError,
    #[error("cannot get messages: feature not available")]
    GetMessagesNotAvailableError,
    #[error("cannot peek messages: feature not available")]
    PeekMessagesNotAvailableError,
    #[error("cannot copy messages: feature not available")]
    CopyMessagesNotAvailableError,
    #[error("cannot move messages: feature not available")]
    MoveMessagesNotAvailableError,
    #[error("cannot delete messages: feature not available")]
    DeleteMessagesNotAvailableError,
}

/// Optional dynamic boxed backend feature.
pub type BackendFeature<F> = Option<Box<F>>;

/// Thread-safe backend feature builder.
///
/// The backend feature builder is a function that takes a reference
/// to a context in parameter and return an optional dynamic boxed
/// backend feature.
pub struct BackendFeatureBuilder<C, F: ?Sized>(
    Option<Arc<dyn Fn(&C) -> BackendFeature<F> + Send + Sync>>,
);

impl<C, F: ?Sized> Clone for BackendFeatureBuilder<C, F> {
    fn clone(&self) -> Self {
        match self.0 {
            None => Self(None),
            Some(ref arc) => Self(Some(arc.clone())),
        }
    }
}

impl<C, F: ?Sized> BackendFeatureBuilder<C, F> {
    pub fn new(func: impl Fn(&C) -> BackendFeature<F> + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(func)))
    }

    pub fn none() -> Self {
        Self(None)
    }

    pub fn into_option(self) -> Option<Arc<dyn Fn(&C) -> BackendFeature<F> + Send + Sync>> {
        self.0
    }

    /// Call this builder function to build this feature.
    /// The given context argument will be passed to the function.
    ///
    /// If you have a more general context than `C` (that can be mapped to `C`) you can use
    /// [`BackendFeatureBuilder::fit_to_context_type`] to map this
    /// `BackendFeatureBuilder`` to another `BackendFeatureBuilder` that can accept that.
    ///
    /// See: [`FindBackendSubcontext`].
    pub fn build_into_feature(self, context: &C) -> BackendFeature<F> {
        self.0.and_then(|f| f(context))
    }
}

impl<C: BackendContext + 'static, F: ?Sized + 'static> BackendFeatureBuilder<C, F> {
    /// Changes `self` from `BackendFeatureBuilder<C, F>` to `BackendFeatureBuilder<B, F>`
    /// where `B: FindBackendSubcontext<C>`
    ///
    /// Allows you use this builder with a "more generic" context, that can be used
    /// to get the current input context `C` through the trait implementation of [`FindBackendSubcontext`].
    pub fn fit_to_context_type<B: FindBackendSubcontext<C>>(self) -> BackendFeatureBuilder<B, F> {
        BackendFeatureBuilder::<B, F>(self.0.map(
            |f| -> Arc<dyn Fn(&B) -> Option<Box<F>> + Send + Sync + 'static> {
                Arc::new(move |ctx: &B| f(ctx.find_subcontext()?))
            },
        ))
    }
}

impl<C, F: ?Sized> Default for BackendFeatureBuilder<C, F> {
    fn default() -> Self {
        Self::none()
    }
}

impl<C, F: ?Sized> From<Option<Arc<dyn Fn(&C) -> BackendFeature<F> + Send + Sync>>>
    for BackendFeatureBuilder<C, F>
{
    fn from(value: Option<Arc<dyn Fn(&C) -> BackendFeature<F> + Send + Sync>>) -> Self {
        BackendFeatureBuilder(value)
    }
}

impl<C, F: ?Sized, MapperFunc> From<MapperFunc> for BackendFeatureBuilder<C, F>
where
    MapperFunc: Fn(&C) -> BackendFeature<F> + Send + Sync + 'static,
{
    fn from(value: MapperFunc) -> Self {
        BackendFeatureBuilder(Some(Arc::new(value)))
    }
}

/// The backend context trait.
///
/// This is just a marker for other traits. Every backend context
/// needs to implement this trait manually or to derive
/// [`BackendContext`].
pub trait BackendContext: Send + Sync {}

/// Get a context in a context.
///
/// A good use case is when you have a custom backend context composed
/// of multiple subcontexts:
///
/// ```rust
/// struct MyContext {
///     imap: email::imap::ImapContextSync,
///     smtp: email::smtp::SmtpContextSync,
/// }
/// ```
///
/// If your context is composed of optional subcontexts, use
/// [`FindBackendSubcontext`] instead.
pub trait GetBackendSubcontext<C: BackendContext> {
    fn get_subcontext(&self) -> &C;
}

/// Generic implementation for contexts that match themselves as
/// subcontext.
impl<C: BackendContext> GetBackendSubcontext<C> for C {
    fn get_subcontext(&self) -> &C {
        self
    }
}

/// Find a context in a context.
///
/// A good use case is when you have a custom backend context composed
/// of multiple optional subcontexts:
///
/// ```rust
/// struct MyContext {
///     imap: Option<email::imap::ImapContextSync>,
///     smtp: Option<email::smtp::SmtpContextSync>,
/// }
/// ```
///
/// If your context is composed of existing subcontexts, use
/// [`GetBackendSubcontext`] instead.
pub trait FindBackendSubcontext<C: BackendContext> {
    fn find_subcontext(&self) -> Option<&C>;
}

/// Generic implementation for contexts that match themselves as
/// subcontext.
///
/// If a context can get a subcontext, then it can also find a
/// subcontext.
impl<C: BackendContext, T: GetBackendSubcontext<C>> FindBackendSubcontext<C> for T {
    fn find_subcontext(&self) -> Option<&C> {
        Some(self.get_subcontext())
    }
}

macro_rules! map_feature_from {
    ($name:ty) => {
        paste! {
            fn [< $name:snake _from >] (
                &self,
                cb: Option<&B>,
            ) -> BackendFeatureBuilder<Self::Context, dyn $name> {
                self.map_feature(BackendContextBuilder::[< $name:snake >], cb)
            }
        }
    };
}

/// Map a feature from a subcontext to a context.
///
/// A good use case is when you have a custom backend context composed
/// of multiple subcontexts. When implementing the
/// [`BackendContextBuilder`] trait for your custom backend context,
/// you will have to forward backend features using the right
/// subcontext.
///
/// ```rust
/// use std::sync::Arc;
/// use async_trait::async_trait;
///
/// use email::imap::{ImapContextSync, ImapContextBuilder};
/// use email::smtp::{SmtpContextSync, SmtpContextBuilder};
/// use email::backend::{BackendContextBuilder, FindBackendSubcontext, BackendFeatureBuilder, MapBackendFeature, macros::BackendContext};
/// use email::account::config::AccountConfig;
/// use email::folder::list::ListFolders;
/// use email::Result;
///
/// #[derive(BackendContext)]
/// struct MyContext {
///     imap: Option<ImapContextSync>,
///     smtp: Option<SmtpContextSync>,
/// }
///
/// impl FindBackendSubcontext<ImapContextSync> for MyContext {
///     fn find_subcontext(&self) -> Option<&ImapContextSync> {
///         self.imap.as_ref()
///     }
/// }
///
/// impl FindBackendSubcontext<SmtpContextSync> for MyContext {
///     fn find_subcontext(&self) -> Option<&SmtpContextSync> {
///         self.smtp.as_ref()
///     }
/// }
///
/// #[derive(Clone)]
/// struct MyContextBuilder {
///     imap: Option<ImapContextBuilder>,
///     smtp: Option<SmtpContextBuilder>,
/// }
///
/// #[async_trait]
/// impl BackendContextBuilder for MyContextBuilder {
///     type Context = MyContext;
///
///     fn list_folders(&self) -> BackendFeatureBuilder<Self::Context, dyn ListFolders> {
///         // This is how you can map a
///         // `BackendFeatureBuilder<ImapContextSync, dyn ListFolders>` to a
///         // `BackendFeatureBuilder<Self::Context, dyn ListFolders>`:
///         self.list_folders_from(self.imap.as_ref())
///     }
///
///     async fn build(self, account_config: Arc<AccountConfig>) -> Result<Self::Context> {
///         let imap = match self.imap {
///             Some(imap) => Some(imap.build(account_config.clone()).await?),
///             None => None,
///         };
///
///         let smtp = match self.smtp {
///             Some(smtp) => Some(smtp.build(account_config).await?),
///             None => None,
///         };
///
///         Ok(MyContext { imap, smtp })
///     }
/// }
/// ```
///
pub trait MapBackendFeature<B>
where
    Self: BackendContextBuilder,
    Self::Context: FindBackendSubcontext<B::Context> + 'static,
    B: BackendContextBuilder,
    B::Context: BackendContext + 'static,
{
    fn map_feature<T: ?Sized + 'static>(
        &self,
        f: impl Fn(&B) -> BackendFeatureBuilder<B::Context, T>,
        cb: Option<&B>,
    ) -> BackendFeatureBuilder<Self::Context, T> {
        BackendFeatureBuilder::<B::Context, T>::from(cb.and_then(|cb| f(cb).into_option()))
            .fit_to_context_type()
    }

    map_feature_from!(AddFolder);
    map_feature_from!(ListFolders);
    map_feature_from!(ExpungeFolder);
    map_feature_from!(PurgeFolder);
    map_feature_from!(DeleteFolder);
    map_feature_from!(GetEnvelope);
    map_feature_from!(ListEnvelopes);
    map_feature_from!(WatchEnvelopes);
    map_feature_from!(AddFlags);
    map_feature_from!(SetFlags);
    map_feature_from!(RemoveFlags);
    map_feature_from!(AddMessage);
    map_feature_from!(SendMessage);
    map_feature_from!(GetMessages);
    map_feature_from!(PeekMessages);
    map_feature_from!(CopyMessages);
    map_feature_from!(MoveMessages);
    map_feature_from!(DeleteMessages);
}

/// Generic implementation for the backend context builder with a
/// context implementing [`FindBackendSubcontext`].
impl<T, B> MapBackendFeature<B> for T
where
    T: BackendContextBuilder,
    T::Context: FindBackendSubcontext<B::Context> + 'static,
    B: BackendContextBuilder,
    B::Context: BackendContext + 'static,
{
    //
}

macro_rules! context_feature {
    ($type:ty) => {
        paste! {
            fn [< $type:snake >] (&self) -> BackendFeatureBuilder<Self::Context, dyn $type> {
                BackendFeatureBuilder::none()
            }
        }
    };
}

/// The backend context builder trait.
///
/// This trait defines how a context should be built. It also defines
/// default backend features implemented by the context.
#[async_trait]
pub trait BackendContextBuilder: Clone + Send + Sync {
    /// The type of the context being built by the builder.
    ///
    /// The context needs to implement [`Send`], as it is sent accross
    /// asynchronous tasks. Wrapping your context in a
    /// [`std::sync::Arc`] should be enough. If your context needs to
    /// be mutated, you can also wrap it in a
    /// [`tokio::sync::Mutex`]. See existing implementations of
    /// `email::imap::ImapContextSync` or
    /// `email::smtp::SmtpContextSync`.
    type Context: BackendContext;

    context_feature!(AddFolder);
    context_feature!(ListFolders);
    context_feature!(ExpungeFolder);
    context_feature!(PurgeFolder);
    context_feature!(DeleteFolder);
    context_feature!(ListEnvelopes);
    context_feature!(WatchEnvelopes);
    context_feature!(GetEnvelope);
    context_feature!(AddFlags);
    context_feature!(SetFlags);
    context_feature!(RemoveFlags);
    context_feature!(AddMessage);
    context_feature!(SendMessage);
    context_feature!(PeekMessages);
    context_feature!(GetMessages);
    context_feature!(CopyMessages);
    context_feature!(MoveMessages);
    context_feature!(DeleteMessages);

    /// Build the final context.
    async fn build(self, account_config: Arc<AccountConfig>) -> Result<Self::Context>;
}

/// Describes to [`BackendBuilder`] how a feature should be initialized.
pub enum FeatureConfiguration<C: BackendContext, T: ?Sized> {
    /// The feature should be initialized in [`Backend`] to `None`
    FeatureDisabled,
    /// The feature should be initialized from the [`BackendContext`].
    /// If the context doesn't support this feature it will be initialized to `None`.
    Default,
    /// Use this given [`BackendFeatureBuilder`] to initialize the feature in [`Backend`].
    /// If this is a `BackendFeatureBuilder::none()`, it will try to initialize this feature
    /// from the context (as if [`FeatureConfiguration::Default`] was used).
    Override(BackendFeatureBuilder<C, T>),
}

// derive clone can't be used https://www.reddit.com/r/rust/comments/y359hf/comment/is7hb1s/?utm_source=share&utm_medium=web3x&utm_name=web3xcss&utm_term=1&utm_content=share_button
impl<C: BackendContext, T: ?Sized> Clone for FeatureConfiguration<C, T> {
    fn clone(&self) -> Self {
        match self {
            Self::FeatureDisabled => Self::FeatureDisabled,
            Self::Default => Self::Default,
            Self::Override(cloneable) => {
                Self::Override(BackendFeatureBuilder::<C, T>::clone(cloneable))
            }
        }
    }
}

impl<C: BackendContext, T: ?Sized> std::fmt::Debug for FeatureConfiguration<C, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FeatureDisabled => write!(f, "FeatureDisabled"),
            Self::Default => write!(f, "Default"),
            Self::Override(_) => write!(f, "Override(...)"),
        }
    }
}

impl<C: BackendContext, T: ?Sized> From<BackendFeatureBuilder<C, T>>
    for FeatureConfiguration<C, T>
{
    fn from(value: BackendFeatureBuilder<C, T>) -> Self {
        Self::Override(value)
    }
}

impl<C: BackendContext, T: ?Sized, F> From<F> for FeatureConfiguration<C, T>
where
    F: Fn(&C) -> BackendFeature<T> + Send + Sync + 'static,
{
    fn from(value: F) -> Self {
        Self::Override(value.into())
    }
}

impl<C: BackendContext, T: ?Sized> FeatureConfiguration<C, T> {
    pub fn with_feature_disabled() -> Self {
        FeatureConfiguration::FeatureDisabled
    }
    pub fn override_with(context_default: impl Into<BackendFeatureBuilder<C, T>>) -> Self {
        FeatureConfiguration::Override(context_default.into())
    }

    pub fn build_into_builder(
        self,
        context_default: &BackendFeatureBuilder<C, T>,
    ) -> BackendFeatureBuilder<C, T> {
        match self {
            FeatureConfiguration::FeatureDisabled => BackendFeatureBuilder::none(),
            FeatureConfiguration::Default => context_default.clone(),
            FeatureConfiguration::Override(bfb) => bfb
                .into_option()
                .or(context_default.clone().into_option())
                .into(),
        }
    }
}

impl<C: BackendContext, T: ?Sized> Default for FeatureConfiguration<C, T> {
    fn default() -> Self {
        Self::Default
    }
}

/// The runtime backend builder.
///
/// The determination of backend's features occurs dynamically at
/// runtime, making the utilization of traits and generics potentially
/// less advantageous in this context. This consideration is
/// particularly relevant if the client interface is an interactive
/// shell (To Be Announced).
///
/// Furthermore, this design empowers the programmatic management of
/// features during runtime.
///
/// Alternatively, users have the option to define their custom
/// structs and implement the same traits as those implemented by
/// `BackendBuilder`. This approach allows for the creation of bespoke
/// functionality tailored to specific requirements.
pub struct BackendBuilder<B: BackendContextBuilder> {
    /// The account configuration.
    pub account_config: Arc<AccountConfig>,

    /// The backend context builder.
    ctx_builder: B,

    /// The [`FeatureConfiguration`]s for each feature.
    features: FeaturesConfiguration<B::Context>,
}

/// This holds a [`FeatureConfiguration`] for each backend feature.
/// This is used by [`BackendBuilder`] to initialize each feature when building a [`Backend`].
#[derive(Debug)]
pub struct FeaturesConfiguration<C: BackendContext> {
    /// The add folder backend feature builder.
    pub add_folder: FeatureConfiguration<C, dyn AddFolder>,

    /// The list folders backend feature builder.
    pub list_folders: FeatureConfiguration<C, dyn ListFolders>,

    /// The expunge folder backend feature builder.
    pub expunge_folder: FeatureConfiguration<C, dyn ExpungeFolder>,

    /// The purge folder backend feature builder.
    pub purge_folder: FeatureConfiguration<C, dyn PurgeFolder>,

    /// The delete folder backend feature builder.
    pub delete_folder: FeatureConfiguration<C, dyn DeleteFolder>,

    /// The list envelopes backend feature builder.
    pub list_envelopes: FeatureConfiguration<C, dyn ListEnvelopes>,

    /// The watch envelopes backend feature builder.
    pub watch_envelopes: FeatureConfiguration<C, dyn WatchEnvelopes>,

    /// The get envelope backend feature builder.
    pub get_envelope: FeatureConfiguration<C, dyn GetEnvelope>,

    /// The add flags backend feature builder.
    pub add_flags: FeatureConfiguration<C, dyn AddFlags>,

    /// The set flags backend feature builder.
    pub set_flags: FeatureConfiguration<C, dyn SetFlags>,

    /// The remove flags backend feature builder.
    pub remove_flags: FeatureConfiguration<C, dyn RemoveFlags>,

    /// The add message backend feature builder.
    pub add_message: FeatureConfiguration<C, dyn AddMessage>,

    /// The send message backend feature builder.
    pub send_message: FeatureConfiguration<C, dyn SendMessage>,

    /// The peek messages backend feature builder.
    pub peek_messages: FeatureConfiguration<C, dyn PeekMessages>,

    /// The get messages backend feature builder.
    pub get_messages: FeatureConfiguration<C, dyn GetMessages>,

    /// The copy messages backend feature builder.
    pub copy_messages: FeatureConfiguration<C, dyn CopyMessages>,

    /// The move messages backend feature builder.
    pub move_messages: FeatureConfiguration<C, dyn MoveMessages>,

    /// The delete messages backend feature builder.
    pub delete_messages: FeatureConfiguration<C, dyn DeleteMessages>,
}

impl<C: BackendContext> FeaturesConfiguration<C> {
    pub fn disabled() -> Self {
        Self {
            add_folder: FeatureConfiguration::FeatureDisabled,
            list_folders: FeatureConfiguration::FeatureDisabled,
            expunge_folder: FeatureConfiguration::FeatureDisabled,
            purge_folder: FeatureConfiguration::FeatureDisabled,
            delete_folder: FeatureConfiguration::FeatureDisabled,
            list_envelopes: FeatureConfiguration::FeatureDisabled,
            watch_envelopes: FeatureConfiguration::FeatureDisabled,
            get_envelope: FeatureConfiguration::FeatureDisabled,
            add_flags: FeatureConfiguration::FeatureDisabled,
            set_flags: FeatureConfiguration::FeatureDisabled,
            remove_flags: FeatureConfiguration::FeatureDisabled,
            add_message: FeatureConfiguration::FeatureDisabled,
            send_message: FeatureConfiguration::FeatureDisabled,
            peek_messages: FeatureConfiguration::FeatureDisabled,
            get_messages: FeatureConfiguration::FeatureDisabled,
            copy_messages: FeatureConfiguration::FeatureDisabled,
            move_messages: FeatureConfiguration::FeatureDisabled,
            delete_messages: FeatureConfiguration::FeatureDisabled,
        }
    }
}

// derive clone can't be used https://www.reddit.com/r/rust/comments/y359hf/comment/is7hb1s/?utm_source=share&utm_medium=web3x&utm_name=web3xcss&utm_term=1&utm_content=share_button
impl<C: BackendContext> Clone for FeaturesConfiguration<C> {
    fn clone(&self) -> Self {
        Self {
            add_folder: self.add_folder.clone(),
            list_folders: self.list_folders.clone(),
            expunge_folder: self.expunge_folder.clone(),
            purge_folder: self.purge_folder.clone(),
            delete_folder: self.delete_folder.clone(),
            list_envelopes: self.list_envelopes.clone(),
            watch_envelopes: self.watch_envelopes.clone(),
            get_envelope: self.get_envelope.clone(),
            add_flags: self.add_flags.clone(),
            set_flags: self.set_flags.clone(),
            remove_flags: self.remove_flags.clone(),
            add_message: self.add_message.clone(),
            send_message: self.send_message.clone(),
            peek_messages: self.peek_messages.clone(),
            get_messages: self.get_messages.clone(),
            copy_messages: self.copy_messages.clone(),
            move_messages: self.move_messages.clone(),
            delete_messages: self.delete_messages.clone(),
        }
    }
}

impl<C: BackendContext> Default for FeaturesConfiguration<C> {
    fn default() -> Self {
        Self {
            add_folder: FeatureConfiguration::default(),
            list_folders: FeatureConfiguration::default(),
            expunge_folder: FeatureConfiguration::default(),
            purge_folder: FeatureConfiguration::default(),
            delete_folder: FeatureConfiguration::default(),
            list_envelopes: FeatureConfiguration::default(),
            watch_envelopes: FeatureConfiguration::default(),
            get_envelope: FeatureConfiguration::default(),
            add_flags: FeatureConfiguration::default(),
            set_flags: FeatureConfiguration::default(),
            remove_flags: FeatureConfiguration::default(),
            add_message: FeatureConfiguration::default(),
            send_message: FeatureConfiguration::default(),
            peek_messages: FeatureConfiguration::default(),
            get_messages: FeatureConfiguration::default(),
            copy_messages: FeatureConfiguration::default(),
            move_messages: FeatureConfiguration::default(),
            delete_messages: FeatureConfiguration::default(),
        }
    }
}

macro_rules! feature_config {
    ($type:ty) => {
        paste! {
            pub fn [<set_ $type:snake>](
                &mut self,
                f: impl Into<FeatureConfiguration<B::Context, dyn $type>>,
            ) {
                self.features.[<$type:snake>] = f.into();
            }

            pub fn [<with_ $type:snake>](
                mut self,
                f: impl Into<FeatureConfiguration<B::Context, dyn $type>>,
            ) -> Self {
                self.[<set_ $type:snake>](f);
                self
            }

            pub fn [<with_disabled_ $type:snake>](
                mut self,
            ) -> Self {
                self.[<set_ $type:snake>](FeatureConfiguration::FeatureDisabled);
                self
            }

            pub fn [<with_context_default_ $type:snake>](
                mut self,
            ) -> Self {
                self.[<set_ $type:snake>](FeatureConfiguration::Default);
                self
            }
        }
    };
}

impl<B: BackendContextBuilder> BackendBuilder<B> {
    /// Build a new backend builder using the given backend context
    /// builder.
    ///
    /// All features are disabled by default.
    pub fn new(account_config: Arc<AccountConfig>, ctx_builder: B) -> Self {
        Self {
            account_config,
            ctx_builder,

            features: FeaturesConfiguration::default(),
        }
    }

    pub fn with_default_features_disabled(mut self) -> Self {
        self.features = FeaturesConfiguration::disabled();

        self
    }

    pub fn with_features_configuration(
        mut self,
        features: FeaturesConfiguration<B::Context>,
    ) -> Self {
        self.features = features;

        self
    }

    feature_config!(AddFolder);
    feature_config!(ListFolders);
    feature_config!(ExpungeFolder);
    feature_config!(PurgeFolder);
    feature_config!(DeleteFolder);
    feature_config!(ListEnvelopes);
    feature_config!(WatchEnvelopes);
    feature_config!(GetEnvelope);
    feature_config!(AddFlags);
    feature_config!(SetFlags);
    feature_config!(RemoveFlags);
    feature_config!(AddMessage);
    feature_config!(SendMessage);
    feature_config!(PeekMessages);
    feature_config!(GetMessages);
    feature_config!(CopyMessages);
    feature_config!(MoveMessages);
    feature_config!(DeleteMessages);

    /// Build the final backend.
    pub async fn build(self) -> Result<Backend<B::Context>> {
        let context = self
            .ctx_builder
            .clone()
            .build(self.account_config.clone())
            .await?;
        let mut backend = Backend::new(self.account_config, context);

        macro_rules! build_feature {
            ($name:tt) => {
                paste! {
                    backend.[<set_ $name>](
                        self
                        .features
                        .$name
                        .build_into_builder(&self.ctx_builder.$name())
                        .build_into_feature(&backend.context)
                    );
                }
            };
        }

        build_feature!(add_folder);
        build_feature!(list_folders);
        build_feature!(expunge_folder);
        build_feature!(purge_folder);
        build_feature!(delete_folder);
        build_feature!(list_envelopes);
        build_feature!(watch_envelopes);
        build_feature!(get_envelope);
        build_feature!(add_flags);
        build_feature!(set_flags);
        build_feature!(remove_flags);
        build_feature!(add_message);
        build_feature!(send_message);
        build_feature!(peek_messages);
        build_feature!(get_messages);
        build_feature!(copy_messages);
        build_feature!(move_messages);
        build_feature!(delete_messages);

        Ok(backend)
    }
}

impl<B: BackendContextBuilder + Clone> Clone for BackendBuilder<B> {
    fn clone(&self) -> Self {
        Self {
            account_config: self.account_config.clone(),
            ctx_builder: self.ctx_builder.clone(),
            features: self.features.clone(),
        }
    }
}

/// The email backend.
///
/// The backend owns a context, as well as multiple optional backend
/// features.
pub struct Backend<C: BackendContext> {
    /// The account configuration.
    pub account_config: Arc<AccountConfig>,

    /// The backend context.
    pub context: C,

    /// The optional add folder feature.
    pub add_folder: BackendFeature<dyn AddFolder>,

    /// The optional list folders feature.
    pub list_folders: BackendFeature<dyn ListFolders>,

    /// The optional expunge folder feature.
    pub expunge_folder: BackendFeature<dyn ExpungeFolder>,

    /// The optional purge folder feature.
    pub purge_folder: BackendFeature<dyn PurgeFolder>,

    /// The optional delete folder feature.
    pub delete_folder: BackendFeature<dyn DeleteFolder>,

    /// The optional list envelopes feature.
    pub list_envelopes: BackendFeature<dyn ListEnvelopes>,

    /// The optional watch envelopes feature.
    pub watch_envelopes: BackendFeature<dyn WatchEnvelopes>,

    /// The optional get envelope feature.
    pub get_envelope: BackendFeature<dyn GetEnvelope>,

    /// The optional add flags feature.
    pub add_flags: BackendFeature<dyn AddFlags>,

    /// The optional set flags feature.
    pub set_flags: BackendFeature<dyn SetFlags>,

    /// The optional remove flags feature.
    pub remove_flags: BackendFeature<dyn RemoveFlags>,

    /// The optional add message feature.
    pub add_message: BackendFeature<dyn AddMessage>,

    /// The optional send message feature.
    pub send_message: BackendFeature<dyn SendMessage>,

    /// The optional peek messages feature.
    pub peek_messages: BackendFeature<dyn PeekMessages>,

    /// The optional get messages feature.
    pub get_messages: BackendFeature<dyn GetMessages>,

    /// The optional copy messages feature.
    pub copy_messages: BackendFeature<dyn CopyMessages>,

    /// The optional move messages feature.
    pub move_messages: BackendFeature<dyn MoveMessages>,

    /// The optional delete messages feature.
    pub delete_messages: BackendFeature<dyn DeleteMessages>,
}

impl<C: BackendContext> Backend<C> {
    /// Build a new backend from an account configuration and a
    /// context.
    pub fn new(account_config: Arc<AccountConfig>, context: C) -> Self {
        Self {
            account_config,
            context,

            add_folder: None,

            list_folders: None,

            expunge_folder: None,

            purge_folder: None,

            delete_folder: None,

            list_envelopes: None,

            watch_envelopes: None,

            get_envelope: None,

            add_flags: None,

            set_flags: None,

            remove_flags: None,

            add_message: None,

            send_message: None,

            peek_messages: None,

            get_messages: None,

            copy_messages: None,

            move_messages: None,

            delete_messages: None,
        }
    }

    /// Set the add folder backend feature.

    pub fn set_add_folder(&mut self, f: BackendFeature<dyn AddFolder>) {
        self.add_folder = f;
    }

    /// Set the list folders backend feature.

    pub fn set_list_folders(&mut self, f: BackendFeature<dyn ListFolders>) {
        self.list_folders = f;
    }
    /// Set the expunge folder backend feature.

    pub fn set_expunge_folder(&mut self, f: BackendFeature<dyn ExpungeFolder>) {
        self.expunge_folder = f;
    }

    /// Set the purge folder backend feature.

    pub fn set_purge_folder(&mut self, f: BackendFeature<dyn PurgeFolder>) {
        self.purge_folder = f;
    }

    /// Set the delete folder backend feature.

    pub fn set_delete_folder(&mut self, f: BackendFeature<dyn DeleteFolder>) {
        self.delete_folder = f;
    }

    /// Set the list envelopes backend feature.

    pub fn set_list_envelopes(&mut self, f: BackendFeature<dyn ListEnvelopes>) {
        self.list_envelopes = f;
    }

    /// Set the watch envelopes backend feature.

    pub fn set_watch_envelopes(&mut self, f: BackendFeature<dyn WatchEnvelopes>) {
        self.watch_envelopes = f;
    }

    /// Set the get envelope backend feature.

    pub fn set_get_envelope(&mut self, f: BackendFeature<dyn GetEnvelope>) {
        self.get_envelope = f;
    }

    /// Set the add flags backend feature.

    pub fn set_add_flags(&mut self, f: BackendFeature<dyn AddFlags>) {
        self.add_flags = f;
    }

    /// Set the set flags backend feature.

    pub fn set_set_flags(&mut self, f: BackendFeature<dyn SetFlags>) {
        self.set_flags = f;
    }

    /// Set the remove flags backend feature.

    pub fn set_remove_flags(&mut self, f: BackendFeature<dyn RemoveFlags>) {
        self.remove_flags = f;
    }

    /// Set the add message backend feature.

    pub fn set_add_message(&mut self, f: BackendFeature<dyn AddMessage>) {
        self.add_message = f;
    }

    /// Set the send message backend feature.

    pub fn set_send_message(&mut self, f: BackendFeature<dyn SendMessage>) {
        self.send_message = f;
    }

    /// Set the peek messages backend feature.

    pub fn set_peek_messages(&mut self, f: BackendFeature<dyn PeekMessages>) {
        self.peek_messages = f;
    }

    /// Set the get messages backend feature.

    pub fn set_get_messages(&mut self, f: BackendFeature<dyn GetMessages>) {
        self.get_messages = f;
    }

    /// Set the copy messages backend feature.

    pub fn set_copy_messages(&mut self, f: BackendFeature<dyn CopyMessages>) {
        self.copy_messages = f;
    }

    /// Set the move messages backend feature.

    pub fn set_move_messages(&mut self, f: BackendFeature<dyn MoveMessages>) {
        self.move_messages = f;
    }

    /// Set the delete messages backend feature.

    pub fn set_delete_messages(&mut self, f: BackendFeature<dyn DeleteMessages>) {
        self.delete_messages = f;
    }

    /// Call the add folder feature, returning an error if the feature
    /// is not defined.

    pub async fn add_folder(&self, folder: &str) -> Result<()> {
        self.add_folder
            .as_ref()
            .ok_or(Error::AddFolderNotAvailableError)?
            .add_folder(folder)
            .await
    }

    /// Call the list folders feature, returning an error if the
    /// feature is not defined.

    pub async fn list_folders(&self) -> Result<Folders> {
        self.list_folders
            .as_ref()
            .ok_or(Error::ListFoldersNotAvailableError)?
            .list_folders()
            .await
    }

    /// Call the expunge folder feature, returning an error if the
    /// feature is not defined.

    pub async fn expunge_folder(&self, folder: &str) -> Result<()> {
        self.expunge_folder
            .as_ref()
            .ok_or(Error::ExpungeFolderNotAvailableError)?
            .expunge_folder(folder)
            .await
    }

    /// Call the purge folder feature, returning an error if the
    /// feature is not defined.

    pub async fn purge_folder(&self, folder: &str) -> Result<()> {
        self.purge_folder
            .as_ref()
            .ok_or(Error::PurgeFolderNotAvailableError)?
            .purge_folder(folder)
            .await
    }

    /// Call the delete folder feature, returning an error if the
    /// feature is not defined.

    pub async fn delete_folder(&self, folder: &str) -> Result<()> {
        self.delete_folder
            .as_ref()
            .ok_or(Error::DeleteFolderNotAvailableError)?
            .delete_folder(folder)
            .await
    }

    /// Call the list envelopes feature, returning an error if the
    /// feature is not defined.

    pub async fn list_envelopes(
        &self,
        folder: &str,
        page_size: usize,
        page: usize,
    ) -> Result<Envelopes> {
        self.list_envelopes
            .as_ref()
            .ok_or(Error::ListEnvelopesNotAvailableError)?
            .list_envelopes(folder, page_size, page)
            .await
    }

    /// Call the watch envelopes feature, returning an error if the
    /// feature is not defined.

    pub async fn watch_envelopes(&self, folder: &str) -> Result<()> {
        self.watch_envelopes
            .as_ref()
            .ok_or(Error::WatchEnvelopesNotAvailableError)?
            .watch_envelopes(folder)
            .await
    }

    /// Call the get envelope feature, returning an error if the
    /// feature is not defined.

    pub async fn get_envelope(&self, folder: &str, id: &Id) -> Result<Envelope> {
        self.get_envelope
            .as_ref()
            .ok_or(Error::GetEnvelopeNotAvailableError)?
            .get_envelope(folder, id)
            .await
    }

    /// Call the add flags feature, returning an error if the feature
    /// is not defined.

    pub async fn add_flags(&self, folder: &str, id: &Id, flags: &Flags) -> Result<()> {
        self.add_flags
            .as_ref()
            .ok_or(Error::AddFlagsNotAvailableError)?
            .add_flags(folder, id, flags)
            .await
    }

    /// Call the add flag feature, returning an error if the feature
    /// is not defined.

    pub async fn add_flag(&self, folder: &str, id: &Id, flag: Flag) -> Result<()> {
        self.add_flags
            .as_ref()
            .ok_or(Error::AddFlagsNotAvailableError)?
            .add_flag(folder, id, flag)
            .await
    }

    /// Call the set flags feature, returning an error if the feature
    /// is not defined.

    pub async fn set_flags(&self, folder: &str, id: &Id, flags: &Flags) -> Result<()> {
        self.set_flags
            .as_ref()
            .ok_or(Error::SetFlagsNotAvailableError)?
            .set_flags(folder, id, flags)
            .await
    }

    /// Call the set flag feature, returning an error if the feature
    /// is not defined.

    pub async fn set_flag(&self, folder: &str, id: &Id, flag: Flag) -> Result<()> {
        self.set_flags
            .as_ref()
            .ok_or(Error::SetFlagsNotAvailableError)?
            .set_flag(folder, id, flag)
            .await
    }

    /// Call the remove flags feature, returning an error if the
    /// feature is not defined.

    pub async fn remove_flags(&self, folder: &str, id: &Id, flags: &Flags) -> Result<()> {
        self.remove_flags
            .as_ref()
            .ok_or(Error::RemoveFlagsNotAvailableError)?
            .remove_flags(folder, id, flags)
            .await
    }

    /// Call the remove flag feature, returning an error if the
    /// feature is not defined.

    pub async fn remove_flag(&self, folder: &str, id: &Id, flag: Flag) -> Result<()> {
        self.remove_flags
            .as_ref()
            .ok_or(Error::RemoveFlagsNotAvailableError)?
            .remove_flag(folder, id, flag)
            .await
    }

    /// Call the add message with flags feature, returning an error if
    /// the feature is not defined.

    pub async fn add_message_with_flags(
        &self,
        folder: &str,
        raw_msg: &[u8],
        flags: &Flags,
    ) -> Result<SingleId> {
        self.add_message
            .as_ref()
            .ok_or(Error::AddMessageWithFlagsNotAvailableError)?
            .add_message_with_flags(folder, raw_msg, flags)
            .await
    }

    /// Call the add message with flag feature, returning an error if
    /// the feature is not defined.

    pub async fn add_message_with_flag(
        &self,
        folder: &str,
        raw_msg: &[u8],
        flag: Flag,
    ) -> Result<SingleId> {
        self.add_message
            .as_ref()
            .ok_or(Error::AddMessageWithFlagsNotAvailableError)?
            .add_message_with_flag(folder, raw_msg, flag)
            .await
    }

    /// Call the add message feature, returning an error if the
    /// feature is not defined.

    pub async fn add_message(&self, folder: &str, raw_msg: &[u8]) -> Result<SingleId> {
        self.add_message
            .as_ref()
            .ok_or(Error::AddMessageWithFlagsNotAvailableError)?
            .add_message(folder, raw_msg)
            .await
    }

    /// Call the send message feature, returning an error if the
    /// feature is not defined.

    pub async fn send_message(&self, msg: &[u8]) -> Result<()> {
        self.send_message
            .as_ref()
            .ok_or(Error::SendMessageNotAvailableError)?
            .send_message(msg)
            .await?;

        if self.account_config.should_save_copy_sent_message() {
            let folder = self.account_config.get_sent_folder_alias();
            log::debug!("saving copy of sent message to {folder}");
            self.add_message_with_flag(&folder, msg, Flag::Seen).await?;
        }

        Ok(())
    }

    /// Call the peek messages feature, returning an error if the
    /// feature is not defined.

    pub async fn peek_messages(&self, folder: &str, id: &Id) -> Result<Messages> {
        self.peek_messages
            .as_ref()
            .ok_or(Error::PeekMessagesNotAvailableError)?
            .peek_messages(folder, id)
            .await
    }

    /// Call the get messages feature, returning an error if the
    /// feature is not defined.

    pub async fn get_messages(&self, folder: &str, id: &Id) -> Result<Messages> {
        self.get_messages
            .as_ref()
            .ok_or(Error::GetMessagesNotAvailableError)?
            .get_messages(folder, id)
            .await
    }

    /// Call the copy messages feature, returning an error if the
    /// feature is not defined.

    pub async fn copy_messages(&self, from_folder: &str, to_folder: &str, id: &Id) -> Result<()> {
        self.copy_messages
            .as_ref()
            .ok_or(Error::CopyMessagesNotAvailableError)?
            .copy_messages(from_folder, to_folder, id)
            .await
    }

    /// Call the move messages feature, returning an error if the
    /// feature is not defined.
    pub async fn move_messages(&self, from_folder: &str, to_folder: &str, id: &Id) -> Result<()> {
        self.move_messages
            .as_ref()
            .ok_or(Error::MoveMessagesNotAvailableError)?
            .move_messages(from_folder, to_folder, id)
            .await
    }

    /// Call the delete messages feature, returning an error if the
    /// feature is not defined.
    pub async fn delete_messages(&self, folder: &str, id: &Id) -> Result<()> {
        self.delete_messages
            .as_ref()
            .ok_or(Error::DeleteMessagesNotAvailableError)?
            .delete_messages(folder, id)
            .await
    }
}
