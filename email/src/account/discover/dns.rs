//! # Account DNS discovery
//!
//! This module contains everything needed to discover account using
//! DNS records.

use hickory_resolver::{
    proto::rr::rdata::{MX, SRV},
    TokioAsyncResolver,
};
use hyper::Uri;
use log::{debug, trace};
use once_cell::sync::Lazy;
use regex::Regex;
use std::{cmp::Ordering, ops::Deref};
use thiserror::Error;

use crate::Result;

/// Regular expression used to extract the URI of a mailconf TXT
/// record.
static MAILCONF_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^mailconf=(https://\S+)$").unwrap());

/// Errors related to account DNS discovery.
#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot find any MX record at {0}")]
    GetMxRecordNotFoundError(String),
    #[error("cannot find any mailconf TXT record at {0}")]
    GetMailconfTxtRecordNotFoundError(String),
    #[error("cannot find any SRV record at {0}")]
    GetSrvRecordNotFoundError(String),
}

/// Sortable wrapper around a MX record.
///
/// This wrapper allows MX records to be sorted by preference.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MxRecord(MX);

impl MxRecord {
    pub fn new(record: MX) -> Self {
        Self(record)
    }
}

impl Deref for MxRecord {
    type Target = MX;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Ord for MxRecord {
    fn cmp(&self, other: &Self) -> Ordering {
        self.preference().cmp(&other.preference())
    }
}

impl PartialOrd for MxRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}

/// Sortable wrapper around a SRV record.
///
/// This wrapper allows MX records to be sorted by priority then
/// weight.
#[derive(Debug, Clone, Eq, PartialEq)]
struct SrvRecord(SRV);

impl SrvRecord {
    pub fn new(record: SRV) -> Self {
        Self(record)
    }
}

impl Deref for SrvRecord {
    type Target = SRV;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Into<SRV> for SrvRecord {
    fn into(self) -> SRV {
        self.0
    }
}

impl Ord for SrvRecord {
    fn cmp(&self, other: &Self) -> Ordering {
        // sort by priority in ascending order
        let priority_cmp = self.priority().cmp(&other.priority());

        if priority_cmp == Ordering::Equal {
            // sort by weight in descending order
            other.weight().cmp(&self.weight())
        } else {
            priority_cmp
        }
    }
}

impl PartialOrd for SrvRecord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Simple DNS client using the tokio async resolver.
pub struct DnsClient {
    resolver: TokioAsyncResolver,
}

impl DnsClient {
    /// Create a new DNS client using defaults.
    pub fn new() -> Self {
        let resolver = TokioAsyncResolver::tokio(Default::default(), Default::default());
        Self { resolver }
    }

    /// Get the first mailconf URI of MX records from the given
    /// domain.
    ///
    /// First, find the MX exchange domain associated to the given
    /// domain, then find the TXT mailconf URI associated to this MX
    /// exchange domain.
    pub async fn get_mailconf_mx_uri(&self, domain: &str) -> Result<Uri> {
        let domain = self.get_mx_domain(domain).await?;
        self.get_mailconf_txt_uri(&domain).await
    }

    /// Get the first mailconf URI of TXT records from the given
    /// domain.
    pub async fn get_mailconf_txt_uri(&self, domain: &str) -> Result<Uri> {
        let records: Vec<String> = self
            .resolver
            .txt_lookup(domain)
            .await?
            .into_iter()
            .map(|record| record.to_string())
            .collect();

        debug!("{domain}: discovered {} TXT record(s)", records.len());
        trace!("{records:#?}");

        let uri = records
            .into_iter()
            .find_map(|record| {
                MAILCONF_REGEX
                    .captures(&record)
                    .and_then(|captures| captures.get(1))
                    .and_then(|capture| capture.as_str().parse::<Uri>().ok())
            })
            .ok_or_else(|| Error::GetMailconfTxtRecordNotFoundError(domain.to_owned()))?;

        debug!("{domain}: best TXT mailconf URI found: {uri}");

        Ok(uri)
    }

    /// Get the first MX exchange domain from a given domain.
    async fn get_mx_domain(&self, domain: &str) -> Result<String> {
        let mut records: Vec<MxRecord> = self
            .resolver
            .mx_lookup(domain)
            .await?
            .into_iter()
            .map(MxRecord::new)
            .collect();

        records.sort();

        debug!("{domain}: discovered {} MX record(s)", records.len());
        trace!("{records:#?}");

        let record = records
            .into_iter()
            .next()
            .ok_or_else(|| Error::GetMxRecordNotFoundError(domain.to_owned()))?;

        let exchange = record.exchange().trim_to(2).to_string();

        debug!("{domain}: best MX domain found: {exchange}");

        Ok(exchange)
    }

    /// Get the first SRV record from a given domain and subdomain.
    async fn get_srv(&self, domain: &str, subdomain: &str) -> Result<SRV> {
        let domain = format!("_{subdomain}._tcp.{domain}");

        let mut records: Vec<SrvRecord> = self
            .resolver
            .srv_lookup(&domain)
            .await?
            .into_iter()
            .filter(|record| !record.target().is_root())
            .map(SrvRecord::new)
            .collect();

        records.sort();

        debug!("{domain}: discovered {} SRV record(s)", records.len());
        trace!("{records:#?}");

        let record: SRV = records
            .into_iter()
            .next()
            .ok_or_else(|| Error::GetSrvRecordNotFoundError(domain.clone()))?
            .into();

        debug!("{domain}: best SRV record found: {record}");

        Ok(record)
    }

    /// Get the first IMAP SRV record from a given domain.
    pub async fn get_imap_srv(&self, domain: &str) -> Result<SRV> {
        self.get_srv(domain, "imap").await
    }

    /// Get the first IMAPS SRV record from a given domain.
    pub async fn get_imaps_srv(&self, domain: &str) -> Result<SRV> {
        self.get_srv(domain, "imaps").await
    }

    /// Get the first SMTP(S) SRV record from a given domain.
    pub async fn get_submission_srv(&self, domain: &str) -> Result<SRV> {
        self.get_srv(domain, "submission").await
    }
}
