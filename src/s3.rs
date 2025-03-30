use crate::{backend::Backend, backend::File, config::Config};
use anyhow::Context;
use anyhow::{bail, Ok, Result};
use s3::Bucket;

use std::str::FromStr;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use log::info;
use s3::{creds::Credentials, Region};

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

pub struct S3 {
    bucket: Bucket,
}

impl Backend for S3 {
    fn get(&self, key: &str) -> Result<Vec<u8>> {
        Ok(self.bucket.get_object(key)?.to_vec())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let response = self.bucket.delete_object(key)?;
        match response.status_code() {
            // The only valid status code
            // https://docs.aws.amazon.com/AmazonS3/latest/API/API_DeleteObject.html
            204 => {
                Ok(())
            },
            403 => bail!("Deletion failed with error 403: Forbidden. Please check that your credentials allows you to delete files to the S3 bucket"),
            err => bail!("Deletion failed with error code {}", err)
        }
    }

    fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let response = self.bucket.put_object(key, data)?;
        match response.status_code() {
            // The only valid status code
            // https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html
            200 => Ok(()),
            403 => bail!("Upload failed with error 403: Forbidden. Please check that your credentials allows you to upload files to the S3 bucket"),
            err => bail!("Upload failed with error code {}", err)
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<File>> {
        Ok(self
            .bucket
            .list(prefix.to_string(), None)?
            .into_iter()
            .flat_map(|v| v.contents)
            .map(|o| {
                Ok(File {
                    key: o.key,
                    last_modified: OffsetDateTime::parse(&o.last_modified, &Rfc3339)?,
                })
            })
            .collect())?
    }

    fn new(config: &Config) -> Result<Self> {
        let connection_info = ConnectionInfo::new(config)?;
        let bucket = Bucket::new(
            &connection_info.bucket_name,
            connection_info.region,
            connection_info.credentials,
        )?;
        Ok(Self { bucket })
    }
}
