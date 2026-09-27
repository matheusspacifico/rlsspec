use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use rlsspec::config::{self, Config, ConfigError, Source};
use rlsspec::pg::PgError;
use rlsspec::runner::{self, RunError};
use rlsspec::{report, safety, style};

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

    /// Allow connecting to hosts outside localhost and `safety.allowed_hosts`
    #[arg(long, global = true)]
    allow_remote: bool,

    /// Disable coloured output (also off when not a terminal or when NO_COLOR is set)
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run every case in the spec against the database
    Test,
    /// Print which identity × table × operation cells have a case, without running any
    Cover,
    /// Print the version
    Version,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.no_color {
        anstream::ColorChoice::Never.write_global();
    }
    match run(&cli) {
        Ok(code) => code,
        Err(err) => {
            match located(&err).and_then(ConfigError::styled) {
                Some(invalid) => anstream::eprint!("{invalid}"),
                None => {
                    let (error, emphasis) = (style::ERROR, style::EMPHASIS);
                    anstream::eprintln!("{error}error{error:#}{emphasis}:{emphasis:#} {err:#}");
                }
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
        Command::Cover => cover(cli),
    }
}

fn test(cli: &Cli) -> Result<ExitCode> {
    let (config, source) = load(cli)?;
    let report = runner::run(&config, &source)?;
    anstream::print!("{}", report::text::render(&report));
    Ok(ExitCode::from(report.exit_code()))
}

fn cover(cli: &Cli) -> Result<ExitCode> {
    let (config, source) = load(cli)?;
    let coverage = runner::cover(&config, &source)?;
    anstream::print!("{}", report::text::render_cover(&coverage));
    Ok(ExitCode::from(u8::from(coverage.fails())))
}

fn load(cli: &Cli) -> Result<(Config, Source)> {
    let (config, source) = config::load(&cli.config)?;
    safety::check(
        &config.database.url,
        &config.safety.allowed_hosts,
        cli.allow_remote,
    )?;
    Ok((config, source))
}

fn located(err: &anyhow::Error) -> Option<&ConfigError> {
    let invalid = match err.downcast_ref::<RunError>() {
        Some(RunError::Config(invalid))
        | Some(RunError::Pg(PgError::SetupTransactionControl(invalid))) => invalid,
        _ => err.downcast_ref::<ConfigError>()?,
    };
    matches!(invalid, ConfigError::Invalid { .. }).then_some(invalid)
}
