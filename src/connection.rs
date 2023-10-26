use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    result::Result::Err,
    str::FromStr,
};

use s3::{creds::Credentials, Region};

use crate::config::Config;

#[derive(Debug)]
pub struct CredentialsError(pub Box<dyn Error>);

impl Display for CredentialsError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Error fetching AWS credentials")
    }
}

impl Error for CredentialsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

pub struct ConnectionInfo {
    pub region: Region,
    pub credentials: Credentials,
    pub bucket_name: String,
}

impl ConnectionInfo {
    pub fn new(config: Config) -> Result<ConnectionInfo, CredentialsError> {
        let credentials = get_credentials(config.remote_profile)?;
        let region = get_region(
            credentials.clone(),
            config.remote_region,
            config.remote_endpoint,
            &config.remote,
        )?;
        Ok(ConnectionInfo {
            credentials,
            region,
            bucket_name: config.remote,
        })
    }
}

pub fn get_credentials(remote_profile: Option<String>) -> Result<Credentials, CredentialsError> {
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
        .map_err(|err| CredentialsError(Box::new(err)))
}

#[derive(Debug)]
struct MissingRegionError;

impl Error for MissingRegionError {}

impl Display for MissingRegionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write! {f, "Could not find a region/endpoint for the S3 bucket"}
    }
}

pub fn get_region(
    _credentials: Credentials,
    remote_region: Option<String>,
    remote_endpoint: Option<String>,
    _bucket_name: &str,
) -> Result<Region, CredentialsError> {
    // Fetch credentials in that order:
    // - from the environment variables
    // - from AWS profile we we have set one in the config
    // - Anonymous access (only for public buckets)
    Region::from_default_env().or_else(|_| {
        if let Some(remote_endpoint) = remote_endpoint {
            Ok(Region::Custom {
                region: remote_region.unwrap_or(String::new()),
                endpoint: remote_endpoint,
            })
        } else if let Some(remote_region) = remote_region {
            Region::from_str(&remote_region).map_err(|err| (CredentialsError(Box::new(err))))
        } else {
            Err(CredentialsError(Box::new(MissingRegionError {})))
        }
    })
}
