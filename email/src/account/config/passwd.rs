//! Module dedicated to password configuration.
//!
//! This module contains everything related to password configuration.

use log::debug;
use secret::Secret;
use std::{
    io,
    ops::{Deref, DerefMut},
};
use thiserror::Error;

use crate::Result;

/// Errors related to password configuration.
#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot get password from user")]
    GetFromUserError(#[source] io::Error),
    #[error("cannot get password from global keyring")]
    GetFromKeyringError(#[source] secret::Error),
    #[error("cannot save password into global keyring")]
    SetIntoKeyringError(#[source] secret::Error),
    #[error("cannot delete password from global keyring")]
    DeleteError(#[source] secret::Error),
}

/// The password configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[cfg_attr(
    feature = "derive",
    derive(serde::Serialize, serde::Deserialize),
    serde(transparent)
)]
pub struct PasswdConfig(
    #[cfg_attr(
        feature = "derive",
        serde(skip_serializing_if = "Secret::is_undefined")
    )]
    pub Secret,
);

impl Deref for PasswdConfig {
    type Target = Secret;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for PasswdConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl PasswdConfig {
    /// If the current password secret is a keyring entry, delete it.
    pub async fn reset(&mut self) -> Result<()> {
        self.delete().await.map_err(Error::DeleteError)?;
        Ok(())
    }

    /// Define the password only if it does not exist in the keyring.
    pub async fn configure(&mut self, get_passwd: impl Fn() -> io::Result<String>) -> Result<()> {
        match self.find().await {
            Ok(None) => {
                debug!("cannot find imap password from keyring, setting it");
                let passwd = get_passwd().map_err(Error::GetFromUserError)?;
                self.set(passwd).await.map_err(Error::SetIntoKeyringError)?;
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(err) => Err(Error::GetFromKeyringError(err).into()),
        }
    }
}
