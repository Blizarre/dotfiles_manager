use std::str::FromStr;

use log::info;
use s3::{creds::Credentials, Region};

use crate::config::Config;

use anyhow::{Context, Result};

pub struct ConnectionInfo {
    pub region: Region,
    pub credentials: Credentials,
    pub bucket_name: String,
}

impl ConnectionInfo {
    pub fn new(config: &Config) -> Result<ConnectionInfo> {
        let credentials = get_credentials(config.remote_profile.clone())?;
        let region = get_region(
            credentials.clone(),
            config.remote_region.clone(),
            config.remote_endpoint.clone(),
        )?;
        Ok(ConnectionInfo {
            credentials,
            region,
            bucket_name: config.remote.clone(),
        })
    }
}

pub fn get_credentials(remote_profile: Option<String>) -> Result<Credentials> {
    // Fetch credentials in that order:
    // - from the environment variables
    // - from AWS profile we we have set one in the config
    // - Anonymous access (only for public buckets)
    Credentials::from_env()
        .or_else(|_| {
            if let Some(remote_profile) = remote_profile {
                Credentials::from_profile(Some(&remote_profile))
            } else {
                Credentials::anonymous()
            }
        })
        .context("Error getting credentials for the remote")
}

pub fn get_region(
    _credentials: Credentials,
    remote_region: Option<String>,
    remote_endpoint: Option<String>,
) -> Result<Region> {
    // Fetch credentials in that order:
    // - from the environment variables
    // - from AWS profile we we have set one in the config
    // - Anonymous access (only for public buckets)
    Region::from_default_env().or_else(|_| {
        if let Some(remote_endpoint) = remote_endpoint {
            Ok(Region::Custom {
                region: remote_region.unwrap_or_default(),
                endpoint: remote_endpoint,
            })
        } else if let Some(remote_region) = remote_region {
            Region::from_str(&remote_region)
                .with_context(|| format!("Invalid region name {}", remote_region))
        } else {
            info!("Could not find an AWS region. Using the default 'us-east-1'");
            Ok(Region::UsEast1)
        }
    })
}
