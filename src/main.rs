use clap::{Parser, Subcommand};

use config::Config;
use filetime::{self, set_file_times, FileTime};
use home::home_dir;
use path_absolutize::Absolutize;
use s3::{self, error::S3Error, Bucket};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::SystemTime,
};

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use diffy::{self, PatchFormatter};

use crate::connection::ConnectionInfo;

mod config;
mod connection;

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Verbose mode
    #[arg(short, long)]
    verbose: bool,

    #[arg(long)]
    config_file: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Add a new dotfile
    Track {
        /// Local filename
        file_name: PathBuf,
        /// Target file on the remote
        target: Option<String>,
    },
    /// Configure the Repository
    Configure {
        /// Target bucket to store the dotfiles
        bucket: String,
        /// AWS Region where the bucket is located
        region: Option<String>,
        /// Optional aws profile to use to connect to the bucket. If not defined will use environment variables and default to anonymous.
        #[arg(long)]
        profile: Option<String>,
        /// Optional S3 url of the remote endpoint to use to communicate with the bucket. This will override the region
        #[arg(long)]
        endpoint: Option<String>,
    },
    Sync,
}

fn main() {
    let args = Args::parse();

    let home_path = home_dir().expect("Unable to find the home directory");
    let config_file_path = args.config_file.unwrap_or_else(|| {
        let mut config_path = home_path.clone();
        config_path.push(".dots");
        config_path
    });
    let config = config::Config::load(config_file_path.as_path())
        .expect("Error loading the configuration file");

    match args.command {
        Commands::Configure {
            bucket,
            region,
            profile,
            endpoint,
        } => {
            let mut config = Config::default();
            config.remote = bucket;
            config.remote_profile = profile.or(config.remote_profile);
            config.remote_region = region.or(config.remote_region);
            config.remote_endpoint = endpoint.or(config.remote_endpoint);
            config
                .save(config_file_path.as_path())
                .expect("Unable to update the configuration file");
        }
        Commands::Sync => sync(home_path.as_path(), config),
        Commands::Track { file_name, target } => {
            track(file_name.as_path(), &home_path, target, config)
        }
    }
}

fn track(file_path: &Path, root_path: &Path, remote_path: Option<String>, config: config::Config) {
    let connection_info = ConnectionInfo::new(config).expect("Error fetching credentials");
    let file_path = file_path.absolutize().expect("error file path");

    let bucket = Bucket::new(
        &connection_info.bucket_name,
        connection_info.region,
        connection_info.credentials,
    )
    .expect("Error when loading the remote bucket");

    if let Some(remote_path) = remote_path {
        upload_local_file(&file_path, &remote_path, &bucket).expect("Error uploading the file");
        return;
    }

    let file_path = file_path.absolutize().expect("error file path");
    let root_path = Path::new(&root_path)
        .absolutize()
        .expect("error remote path");
    if !file_path.starts_with(&root_path) {
        println!("Error, the file is not inside the root path");
        return;
    }

    let remote_path = file_path
        .strip_prefix(root_path)
        .expect("Cannot remove prefix");

    upload_local_file(
        &file_path,
        remote_path.to_str().expect("Invalid remote path"),
        &bucket,
    )
    .expect("Error uploading the file");
}

fn sync(root_dir: &Path, config: config::Config) {
    let connection_info = ConnectionInfo::new(config).expect("Error fetching credentials");
    let bucket = Bucket::new(
        &connection_info.bucket_name,
        connection_info.region,
        connection_info.credentials,
    )
    .expect("Error when loading the remote bucket");
    println!("Listing files from {}", connection_info.bucket_name);

    let results = bucket.list("".to_string(), Some("/".to_string())).unwrap();

    for result in results {
        for file in result.contents {
            println!("< {}, {}", file.key, file.last_modified);

            let last_modified_s3 = OffsetDateTime::parse(&file.last_modified, &Rfc3339)
                .expect("Error parsing aws s3 header");

            let local = root_dir.join(Path::new(&file.key));
            let object = bucket
                .get_object(&file.key)
                .expect("Could not retrieve file");

            if local.exists() {
                println!("Found a local version");
                let metadata =
                    std::fs::metadata(&local).expect("Could not get metadata for local file");
                let last_modified_local =
                    OffsetDateTime::from(metadata.modified().expect("Could not read datetime"));
                println!(
                    "Conflict: Local file: {}, Remote file: {}",
                    last_modified_local, last_modified_s3
                );
                let local_content =
                    std::fs::read_to_string(&local).expect("Cannot read local content");
                let content_s3 =
                    &String::from_utf8(object.bytes().to_vec()).expect("Error parsing file");
                let patch = diffy::create_patch(&local_content, content_s3);
                if patch.hunks().is_empty() {
                    println!("Identical content, skipping");
                } else {
                    let patch_fmt = PatchFormatter::new().with_color();
                    println!(
                        "Original is local, Modified is remote:\n{}",
                        patch_fmt.fmt_patch(&patch)
                    );
                    let response = ask_user("Upload (u) local version, Overwrite (o) local version with remote, Skip (s) this file, or Exit (e)", vec!["u", "o", "s", "e"]);
                    match response.as_str() {
                        "u" => upload_local_file(&local, &file.key, &bucket)
                            .expect("Error uploading file"),
                        "o" => replace_local_file(
                            &local,
                            object.bytes(),
                            SystemTime::from(last_modified_s3),
                        ),
                        "s" => continue,
                        "e" => return,
                        _ => println!("Unsupported at the moment"),
                    }
                }
            } else {
                println!("local version missing, retrieving");
                replace_local_file(&local, object.bytes(), SystemTime::from(last_modified_s3));
            }
        }
    }
}

fn upload_local_file(file_path: &Path, bucket_key: &str, _bucket: &Bucket) -> Result<(), S3Error> {
    println!(
        "Uploading {} to {}",
        file_path.to_str().expect("ASTRING"),
        bucket_key
    );
    //    let data = std::fs::read(file_path).expect("Error reading file to upload");
    //    bucket.put_object(bucket_key, &data).map(|_| {})
    Ok(())
}

fn replace_local_file(path: &Path, content: &[u8], modified_time: SystemTime) {
    std::fs::write(path, content).expect("Could not write file to disk");
    let last_modified_s3 = FileTime::from_system_time(modified_time);
    set_file_times(path, last_modified_s3, last_modified_s3)
        .expect("Error when updating the time for the downloaded file")
}

fn ask_user(prompt: &str, accepted_values: Vec<&str>) -> String {
    println!("{}", prompt);
    let mut line = String::new();
    while !accepted_values.contains(&line.trim()) {
        print!("input [{}]: ", accepted_values.join(", "));
        std::io::stdout().flush().unwrap_or_default();
        std::io::stdin().read_line(&mut line).unwrap();
    }
    return line.trim().to_owned();
}
