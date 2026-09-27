use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use rlsspec::config::{self, Config, ConfigError, Source};
use rlsspec::init::{self, InitError};
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
    /// Write a spec from the database (DATABASE_URL) with every identity × table × operation `todo`
    Init {
        /// Schemas whose tables are in scope
        #[arg(long, value_delimiter = ',', default_value = "public")]
        schemas: Vec<String>,
        /// Where to write the spec
        #[arg(short, long, default_value = "rlsspec.yaml")]
        output: PathBuf,
        /// Overwrite the output file if it exists
        #[arg(long)]
        force: bool,
    },
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
        Command::Init {
            ref schemas,
            ref output,
            force,
        } => init(cli, schemas, output, force),
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

fn init(cli: &Cli, schemas: &[String], output: &Path, force: bool) -> Result<ExitCode> {
    if !force && output.exists() {
        return Err(InitError::Exists(output.to_path_buf()).into());
    }
    let Ok(url) = env::var("DATABASE_URL") else {
        bail!("`DATABASE_URL` is not set; init reads the database to scaffold the spec from it");
    };
    safety::check(&url, &[], cli.allow_remote)?;
    let scaffold = init::introspect(&url, schemas)?;
    init::write(output, &init::render(&scaffold), force)?;
    println!(
        "wrote {}: {} tables × {} identities, {} cells todo",
        output.display(),
        scaffold.tables.len(),
        scaffold.identities.len(),
        scaffold.cells()
    );
    Ok(ExitCode::SUCCESS)
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
