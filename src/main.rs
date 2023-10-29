use clap::{Parser, Subcommand};

use config::Config;
use filetime::{self, set_file_times, FileTime};
use home::home_dir;
use log::{debug, info};
use path_absolutize::Absolutize;
use s3::{self, Bucket};
use std::{
    collections::HashSet,
    fs::{self, DirEntry},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
    time::SystemTime,
};

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use diffy::{self, PatchFormatter};

use crate::connection::ConnectionInfo;

mod config;
mod connection;

use anyhow::{bail, Context, Ok, Result};

/// dotfile manages your configuration files across your computers.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Verbose mode
    #[arg(short, long)]
    verbose: bool,

    /// Quiet mode
    #[arg(short, long)]
    quiet: bool,

    #[arg(long)]
    config_file: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Add a new dotfile
    Track {
        /// Local filenames or directories
        sources: Vec<PathBuf>,
        /// Target file on the remote
        #[arg(short, long)]
        target: Option<String>,
    },
    /// Configure the Repository and create the configuration file. This can be skipped with environment variables
    Configure {
        /// Target bucket to store the dotfiles (DOT_REMOTE)
        bucket: String,
        /// AWS Region where the bucket is located. us-east-1 by default (DOT_REMOTE_REGION)
        region: Option<String>,
        /// Optional aws profile to use to connect to the bucket. If not defined will use environment variables and default to anonymous (DOT_REMOTE_PROFILE)
        #[arg(short, long)]
        profile: Option<String>,
        /// Optional S3 url of the remote endpoint to use to communicate with the bucket. This will override the region (DOT_REMOTE_ENDPOINT)
        #[arg(long)]
        endpoint: Option<String>,
        /// Root directory on the disk that will be matched with the remote. Default is the home directory (DOT_ROOT_DIR)
        root_dir: Option<String>,
    },
    /// Forget a file in the remote
    Forget { target: String },
    /// Synchronize your local directory with the remote (download changes / upload changes)
    Sync,
    /// List all files tracked by dotfile
    List,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.quiet && args.verbose {
        bail!("--quiet and --verbose cannot be used together");
    }
    if args.verbose {
        simple_logger::init_with_level(log::Level::Debug)
    } else if args.quiet {
        simple_logger::init_with_level(log::Level::Error)
    } else {
        simple_logger::init_with_level(log::Level::Info)
    }?;

    let config_file_path = args
        .config_file.as_ref()
        .map_or_else(|| {
            let mut dir = home_dir().context("Unable to find the home directory to get the config file. You can provide the config file as argument with --config-file-path")?;
            dir.push(".dots");
            Ok::<PathBuf>(dir.to_owned())
        }, |p| Ok(p.to_owned())
    )?;
    let config_file_path = config_file_path.as_path();

    if let Commands::Configure {
        bucket,
        region,
        profile,
        endpoint,
        root_dir,
    } = args.command
    {
        let mut config = Config::default();
        config.root_dir = root_dir.clone();
        config.remote = bucket.clone();
        config.remote_profile = profile.clone().or(config.remote_profile);
        config.remote_region = region.clone().or(config.remote_region);
        config.remote_endpoint = endpoint.clone().or(config.remote_endpoint);
        config
            .save(config_file_path)
            .context("Error saving the config file")?;
        info!("New configuration saved in {}", config_file_path.display());
        return Ok(());
    }

    let config = config::Config::load(config_file_path)?;

    let root_dir = config.root_dir.as_ref()
        .map_or_else(
            || home_dir()
                .context("Unable to find the home directory to use as the root directory. You can set the root directory explicitly in the config file"),
            |p| Ok(PathBuf::from_str(p)?),
        )?;
    let root_dir = &root_dir.as_path();

    match &args.command {
        Commands::Sync => sync(root_dir, &config),
        Commands::Track { sources, target } => track(sources, root_dir, target.clone(), &config),
        Commands::Forget { target } => forget(target, &config),
        Commands::Configure { .. } => Ok(()),
        Commands::List {} => list(&config),
    }
}

fn forget(target: &str, config: &Config) -> Result<()> {
    let connection_info = ConnectionInfo::new(config)?;
    let bucket = Bucket::new(
        &connection_info.bucket_name,
        connection_info.region,
        connection_info.credentials,
    )
    .context("Error when loading the remote bucket")?;

    let response = bucket.delete_object(target)?;

    match response.status_code() {
        // The only valid status code
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_DeleteObject.html
        204 => Ok(()),
        403 => bail!("Deletion failed with error 403: Forbidden. Please check that your credentials allows you to delete files to the S3 bucket"),
        err => bail!("Deletion failed with error code {}", err)
    }
}

fn list(config: &Config) -> Result<()> {
    let connection_info = ConnectionInfo::new(config)?;
    let bucket = Bucket::new(
        &connection_info.bucket_name,
        connection_info.region,
        connection_info.credentials,
    )
    .context("Error when loading the remote bucket")?;

    let results = bucket
        .list("".to_string(), None)
        .context("Could not list the bucket content. It could be an invalid region or endpoint, invalid credentials, or network issues.")?;

    for result in results {
        for file in result.contents {
            println!("{}", file.key);
        }
    }
    Ok(())
}

fn visit_dirs(dir: &Path, cb: &mut dyn FnMut(&DirEntry)) -> Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit_dirs(&path, cb)?;
            } else {
                cb(&entry);
            }
        }
    }
    Ok(())
}

fn track(
    sources: &Vec<PathBuf>,
    root_dir: &Path,
    remote_path: Option<String>,
    config: &config::Config,
) -> Result<()> {
    let connection_info = ConnectionInfo::new(config)?;

    let bucket = Bucket::new(
        &connection_info.bucket_name,
        connection_info.region,
        connection_info.credentials,
    )
    .context("Error when loading the remote bucket")?;

    let mut files: HashSet<PathBuf> = HashSet::new();

    let root_path = Path::new(&root_dir)
        .absolutize()
        .context("Could not find the absolute location of the root path")?;

    for source_path in sources {
        let source_path = source_path
            .absolutize()
            .context("Could not find the absolute location of the input file")?;

        if let Some(remote_path) = remote_path {
            if source_path.is_dir() {
                bail!("The remote path can only be defined if there is a single source file")
            }
            return upload_local_file(&source_path, &remote_path, &bucket);
        }

        if !source_path.starts_with(&root_path) {
            bail!(
                "Error, the file {} is not inside the root path {}",
                source_path.display(),
                root_path.display()
            );
        }

        if source_path.is_dir() {
            visit_dirs(&source_path, &mut |f| {
                files.insert(f.path());
            })?;
        } else {
            files.insert(source_path.to_path_buf());
        };
    }

    for file in files {
        let remote_path = file
            .strip_prefix(&root_path)
            .context("Error when trying to generate the path in the S3 bucket")?;

        upload_local_file(
            &file,
            remote_path.to_str().context("Invalid remote path")?,
            &bucket,
        )?
    }
    Ok(())
}

fn sync(root_dir: &Path, config: &config::Config) -> Result<()> {
    let connection_info = ConnectionInfo::new(config)?;
    let bucket = Bucket::new(
        &connection_info.bucket_name,
        connection_info.region,
        connection_info.credentials,
    )
    .context("Error when loading the remote bucket")?;
    info!("Listing files from {}", connection_info.bucket_name);

    let results = bucket
        .list("".to_string(), None)
        .context("Could not list the bucket content. It could be an invalid region or endpoint, invalid credentials, or network issues.")?;

    for result in results {
        for file in result.contents {
            debug!("Remote: {}, {}", file.key, file.last_modified);

            let last_modified_s3 = OffsetDateTime::parse(&file.last_modified, &Rfc3339)
                .context("Error parsing the file modification date from the aws s3 header")?;

            let local = root_dir.join(Path::new(&file.key));
            let object = bucket
                .get_object(&file.key)
                .with_context(|| format!("Could not retrieve file {} from S3", &file.key))?;

            if local.exists() {
                debug!("    Found matching local file: {}", local.display());
                let metadata = std::fs::metadata(&local)
                    .context("Could not get metadata for the local file")?;
                let last_modified_local = OffsetDateTime::from(
                    metadata
                        .modified()
                        .context("Could not read modification time for the local file")?,
                );
                debug!(
                    "    Conflict: Local file: {}, Remote file: {}",
                    last_modified_local, last_modified_s3
                );
                let local_content = std::fs::read_to_string(&local)
                    .context("Error reading the content of the local file")?;
                let content_s3 = &String::from_utf8(object.bytes().to_vec())
                    .context("The remote file is not a text file")?;
                let patch = diffy::create_patch(&local_content, content_s3);
                if patch.hunks().is_empty() {
                    info!("    Identical content, skipping: {}", file.key);
                } else {
                    let patch_fmt = PatchFormatter::new().with_color();
                    info!(
                        "    {} - Original is local, Modified is remote:\n{}",
                        file.key,
                        patch_fmt.fmt_patch(&patch)
                    );
                    let response = ask_user("Upload (u) local version, Overwrite (o) local version with remote, Skip (s) this file, or Exit (e)", vec!["u", "o", "s", "e"]);
                    match response.as_str() {
                        "u" => upload_local_file(&local, &file.key, &bucket)?,
                        "o" => replace_local_file(
                            &local,
                            object.bytes(),
                            SystemTime::from(last_modified_s3),
                        )?,
                        "s" => continue,
                        "e" => return Ok(()),
                        _ => bail!("Unknown action"),
                    }
                }
            } else {
                info!("    local version missing, retrieving");
                replace_local_file(&local, object.bytes(), SystemTime::from(last_modified_s3))?;
            }
        }
    }
    Ok(())
}

fn upload_local_file(file_path: &Path, bucket_key: &str, bucket: &Bucket) -> Result<()> {
    info!("Uploading {} to {}", file_path.display(), bucket_key);
    let data = std::fs::read(file_path).context("Error reading file to upload")?;
    let response = bucket.put_object(bucket_key, &data).with_context(|| {
        format!(
            "Error uploading file {} to the S3 bucket {}:{}",
            file_path.display(),
            bucket.name,
            bucket_key
        )
    })?;
    // I guess that's a bug from the s3 crate that isn't propagating errors from the http library.
    match response.status_code() {
        // The only valid status code
        // https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html
        200 => Ok(()),
        403 => bail!("Upload failed with error 403: Forbidden. Please check that your credentials allows you to upload files to the S3 bucket"),
        err => bail!("Upload failed with error code {}", err)
    }
}

fn replace_local_file(path: &Path, content: &[u8], modified_time: SystemTime) -> Result<()> {
    std::fs::write(path, content).context("Error updating the local file")?;
    let last_modified_s3 = FileTime::from_system_time(modified_time);
    set_file_times(path, last_modified_s3, last_modified_s3)
        .context("Error when updating the time for the downloaded file")
}

fn ask_user(prompt: &str, accepted_values: Vec<&str>) -> String {
    print!("{}", prompt);
    let mut line = String::new();
    while !accepted_values.contains(&line.trim()) {
        print!("input [{}]: ", accepted_values.join(", "));
        std::io::stdout().flush().unwrap_or_default();
        std::io::stdin().read_line(&mut line).unwrap();
    }
    return line.trim().to_owned();
}
