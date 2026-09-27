use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use rlsspec::config::{self, ConfigError};

const EXIT_ERROR: u8 = 2;

#[derive(Parser)]
#[command(
    name = "rlsspec",
    version,
    about = "Check Postgres Row Level Security against a spec of expected access"
)]
struct Cli {
    #[arg(short, long, global = true, default_value = "rlsspec.yaml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run every case in the spec against the database
    Test,
    /// Print the version
    Version,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(err) => {
            match err.downcast_ref::<ConfigError>() {
                Some(invalid @ ConfigError::Invalid { .. }) => eprint!("{invalid}"),
                _ => eprintln!("error: {err:#}"),
            }
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Version => {
            println!("rlsspec {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        Command::Test => test(cli),
    }
}

fn test(cli: &Cli) -> Result<ExitCode> {
    config::load(&cli.config)?;
    eprintln!("error: running cases is not implemented yet");
    Ok(ExitCode::from(EXIT_ERROR))
}
