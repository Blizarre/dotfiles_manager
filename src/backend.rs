use crate::config::Config;
use crate::ConnectionInfo;
use anyhow::{bail, Ok, Result};
use s3::Bucket;
use std::fmt::Display;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub trait Backend {
    fn get(&self, key: &str) -> Result<Vec<u8>>;
    fn delete(&self, key: &str) -> Result<()>;
    fn put(&self, key: &str, data: &[u8]) -> Result<()>;
    fn list(&self, prefix: &str) -> Result<Vec<File>>;
    fn new(config: &Config) -> Result<Self>
    where
        Self: Sized;
}

pub struct File {
    pub key: String,
    pub last_modified: OffsetDateTime,
}

impl Display for File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.key)
    }
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
