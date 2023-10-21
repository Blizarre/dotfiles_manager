use clap::{Parser, Subcommand};
use home::home_dir;

use std::path::PathBuf;

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
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Add a new dotfile
    Track { file_name: PathBuf },
    /// Configure the Repository
    Configure {
        /// Url of the git repo
        git_repo: String,
    },
}

fn main() {
    let args = Args::parse();
    println!("{:?}", args);

    let config_file_path = args.config_file.unwrap_or_else(|| {
        let mut home_path = home_dir().expect("Unable to find the home directory");
        home_path.push(".dots");
        home_path
    });
    let _config = config::Config::load(config_file_path.as_path())
        .expect("Error loading the configuration file");
}
