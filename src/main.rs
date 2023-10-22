use clap::{Parser, Subcommand};
use filetime::{self, set_file_times, FileTime};
use home::home_dir;
use s3::{self, creds::Credentials, Bucket};

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::SystemTime,
};

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use diffy::{self, PatchFormatter};

mod config;

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
        file_name: PathBuf,
    },
    /// Configure the Repository
    Configure {
        /// Target bucket to store the dotfiles
        bucket: String,
        /// Optional aws profile to use to connect to the bucket. If not defined will use environment variables and default to anonymous.
        profile: Option<String>,
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
        Commands::Configure { bucket, profile } => {
            let mut new_config = config;
            new_config.remote = bucket;
            new_config.remote_profile = profile;
            new_config
                .save(config_file_path.as_path())
                .expect("Unable to update the configuration file");
        }
        Commands::Sync => sync(home_path.as_path(), config),
        _ => {}
    }
}

fn sync(root_dir: &Path, config: config::Config) {
    let region = "eu-west-2".parse().unwrap();

    // Fetch credentials in that order:
    // - from the environment variables
    // - IF NOT FOUND OR INVALID: see if we defined a
    let credentials = if let Ok(credentials) = Credentials::from_env() {
        credentials
    } else if let Some(profile_name) = config.remote_profile {
        Credentials::from_profile(Some(&profile_name))
    } else {
        Credentials::anonymous()
    }
    .expect("Impossible to find credentials");

    let bucket =
        Bucket::new(&config.remote, region, credentials).expect("Error when loading the bucket");
    println!("Listing files from {}", config.remote);

    let files = bucket.list("".to_string(), Some("/".to_string())).unwrap();

    for file in files {
        for f in file.contents {
            println!("< {}, {}", f.key, f.last_modified);

            let last_modified_s3 = OffsetDateTime::parse(&f.last_modified, &Rfc3339)
                .expect("Error parsing aws s3 header");

            let local = root_dir.join(Path::new(&f.key));
            let object = bucket.get_object(f.key).expect("Could not retrieve file");

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
                    let f = PatchFormatter::new().with_color();
                    println!(
                        "Original is local, Modified is remote:\n{}",
                        f.fmt_patch(&patch)
                    );
                    let response = ask_user("Upload (u) local version, Overwrite (o) local version with remote, Skip (s) this file, or Exit (e)", vec!["u", "o", "s", "e"]);
                    match response.as_str() {
                        "u" => upload_local_file(&local, &bucket),
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

fn upload_local_file(p: &Path, bucket: &Bucket) {
    let data = std::fs::read(p).expect("Error reading file to upload");
    bucket
        .put_object(p.to_str().expect("Invalid"), &data)
        .expect("Error uploading file to S3");
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
