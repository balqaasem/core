//! Module dedicated to email folders synchronization.
//!
//! This module contains everything you need to synchronize remote
//! folders with local ones.

pub mod cache;
mod hunk;
mod patch;
mod report;

use std::collections::HashSet;

#[doc(inline)]
pub use self::{
    cache::FolderSyncCache,
    hunk::{FolderName, FolderSyncCacheHunk, FolderSyncHunk, FoldersName},
    patch::{FolderSyncCachePatch, FolderSyncPatch, FolderSyncPatchManager, FolderSyncPatches},
    report::FolderSyncReport,
};

/// The folder synchronization strategy.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum FolderSyncStrategy {
    /// Synchronizes all folders.
    #[default]
    All,

    /// Synchronizes only folders matching the given names.
    Include(HashSet<String>),

    /// Synchronizes all folders except the ones matching the given
    /// names.
    Exclude(HashSet<String>),
}

impl FolderSyncStrategy {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
