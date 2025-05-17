use clap::{Parser, Subcommand};

use backend::Backend;
use filetime::{self, set_file_times, FileTime};
use home::home_dir;
use log::{debug, info};
use path_absolutize::Absolutize;
use std::{
    collections::HashSet,
    fs::{self, DirEntry},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};

use time::OffsetDateTime;

use diffy::{self, PatchFormatter};

mod backend;
mod config;
mod webdav;

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
        /// Target URL (DOT_URL). Should contain scheme and authentication information if
        /// necessary: https://user:password@webdavserver.com/location/of/the/directory/
        /// User and password should be urlencoded.
        url: String,
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
    let mut logger = env_logger::Builder::from_default_env();
    let logger = if args.verbose {
        logger.filter_level(log::LevelFilter::Debug)
    } else if args.quiet {
        logger.filter_level(log::LevelFilter::Error)
    } else {
        &mut logger
    };
    logger.init();

    let config_file_path = args
        .config_file.as_ref()
        .map_or_else(|| {
            let mut dir = home_dir().context("Unable to find the home directory to get the config file. You can provide the config file as argument with --config-file-path")?;
            dir.push(".dots");
            Ok::<PathBuf>(dir.to_owned())
        }, |p| Ok(p.to_owned())
    )?;
    let config_file_path = config_file_path.as_path();

    if let Commands::Configure { url, root_dir } = args.command {
        let config = config::Config {
            url: url.clone(),
            root_dir: root_dir.clone(),
            ..Default::default()
        };
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

    let backend = webdav::Webdav::new(&config)?;

    match &args.command {
        Commands::Sync => sync(root_dir, &backend),
        Commands::Track { sources, target } => track(sources, root_dir, target.clone(), &backend),
        Commands::Forget { target } => forget(target, &backend),
        Commands::Configure { .. } => Ok(()),
        Commands::List => list(&backend),
    }
}

fn forget(target: &str, backend: &dyn Backend) -> Result<()> {
    backend.delete(target)?;
    info!("The file {} has been removed", target);
    Ok(())
}

fn list(backend: &dyn Backend) -> Result<()> {
    let results = backend.list()
        .context("Could not list the remote content. It could be an invalid endpoint, invalid credentials, or network issues.")?;

    for file in results {
        println!("{}", file);
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
    backend: &dyn Backend,
) -> Result<()> {
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
            return upload_local_file(&source_path, &remote_path, backend);
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
            .context("Error when trying to generate the path for the remote endpoint")?;

        upload_local_file(
            &file,
            remote_path.to_str().context("Invalid remote path")?,
            backend,
        )?
    }
    Ok(())
}

fn sync(root_dir: &Path, backend: &dyn Backend) -> Result<()> {
    info!("Listing files");

    let results = backend.list().context("Could not list the remote content. It could be an invalid URL, invalid credentials, or network issues.")?;

    for file in results {
        debug!("Remote: {}, {}", file.key, file.last_modified);

        let local = root_dir.join(Path::new(&file.key));
        let content = backend
            .get(&file.key)
            .with_context(|| format!("Could not retrieve file {}", &file.key))?;

        if local.exists() {
            debug!("    Found matching local file: {}", local.display());
            let metadata =
                std::fs::metadata(&local).context("Could not get metadata for the local file")?;
            let last_modified_local = OffsetDateTime::from(
                metadata
                    .modified()
                    .context("Could not read modification time for the local file")?,
            );
            debug!(
                "    Conflict: Local file: {}, Remote file: {}",
                last_modified_local, file.last_modified
            );
            let local_content = std::fs::read_to_string(&local)
                .context("Error reading the content of the local file")?;
            let content_text = &String::from_utf8(content.clone())
                .context("The remote file is not a text file")?;
            let patch = diffy::create_patch(&local_content, content_text);
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
                    "u" => upload_local_file(&local, &file.key, backend)?,
                    "o" => replace_local_file(&local, &content, file.last_modified)?,
                    "s" => continue,
                    "e" => return Ok(()),
                    _ => bail!("Unknown action"),
                }
            }
        } else {
            info!("    Local version missing, retrieving {}", file.key);
            replace_local_file(&local, &content, file.last_modified)?;
        }
    }
    Ok(())
}

fn upload_local_file(file_path: &Path, path: &str, backend: &dyn Backend) -> Result<()> {
    info!("Uploading {} to {}", file_path.display(), path);
    let data = std::fs::read(file_path).context("Error reading file to upload")?;
    backend
        .put(path, &data)
        .with_context(|| format!("Error uploading file {} at {}", file_path.display(), path))
}

fn replace_local_file(path: &Path, content: &[u8], modified_time: OffsetDateTime) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create intermediate directory {}", parent.display()))?
    }

    std::fs::write(path, content).context("Error updating the local file")?;
    let last_modified = FileTime::from_system_time(modified_time.into());
    set_file_times(path, last_modified, last_modified)
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
    line.trim().to_owned()
}
