use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "rlsspec",
    version,
    about = "Check Postgres Row Level Security against a spec of expected access"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the version
    Version,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Version => println!("rlsspec {}", env!("CARGO_PKG_VERSION")),
    }
    ExitCode::SUCCESS
}
