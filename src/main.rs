use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use rlsspec::config::{self, Config, ConfigError, Source};
use rlsspec::init::{self, InitError};
use rlsspec::lint;
use rlsspec::pg::{PgError, Target};
use rlsspec::preset::Preset;
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

    /// Allow a non-local host without TLS (sslmode=disable, or prefer without upgrading it to require)
    #[arg(long, global = true)]
    allow_insecure: bool,

    /// Disable coloured output (also off when not a terminal or when NO_COLOR is set)
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run every case in the spec against the database
    Test {
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
    /// Print which identity × table × operation cells have a case, without running any
    Cover {
        #[arg(long, value_enum, default_value_t = CoverFormat::Text)]
        format: CoverFormat,
    },
    /// Check the catalog for common RLS mistakes (rules RLS001–RLS008), without running any case
    Lint {
        #[arg(long, value_enum, default_value_t = Format::Text)]
        format: Format,
    },
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
        /// Scaffold the identities of a platform preset (supabase) instead of one per role
        #[arg(long, value_parser = preset)]
        preset: Option<Preset>,
    },
    /// Print the version
    Version,
}

/// How a report is written to stdout. Errors are text on stderr whatever the format.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// Human-readable, coloured on a terminal
    Text,
    /// One JSON document (schema_version 1)
    Json,
    /// JUnit XML, for CI test reporters
    Junit,
}

/// `cover` runs no test, so it has no JUnit report.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CoverFormat {
    /// Human-readable, coloured on a terminal
    Text,
    /// One JSON document (schema_version 1)
    Json,
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
        Command::Test { format } => test(cli, format),
        Command::Cover { format } => cover(cli, format),
        Command::Lint { format } => lint(cli, format),
        Command::Init {
            ref schemas,
            ref output,
            force,
            preset,
        } => init(cli, schemas, output, force, preset),
    }
}

fn test(cli: &Cli, format: Format) -> Result<ExitCode> {
    let (config, source, target) = load(cli)?;
    let report = runner::run(&config, &source, &target)?;
    match format {
        Format::Text => anstream::print!("{}", report::text::render(&report)),
        Format::Json => print!("{}", report::json::render(&report)?),
        Format::Junit => print!("{}", report::junit::render(&report)?),
    }
    Ok(ExitCode::from(report.exit_code()))
}

fn cover(cli: &Cli, format: CoverFormat) -> Result<ExitCode> {
    let (config, source, target) = load(cli)?;
    let coverage = runner::cover(&config, &source, &target)?;
    match format {
        CoverFormat::Text => anstream::print!("{}", report::text::render_cover(&coverage)),
        CoverFormat::Json => print!("{}", report::json::render_cover(&coverage)?),
    }
    Ok(ExitCode::from(u8::from(coverage.fails())))
}

fn lint(cli: &Cli, format: Format) -> Result<ExitCode> {
    let (config, source, target) = load(cli)?;
    let report = lint::run(&config, &source, &target)?;
    match format {
        Format::Text => anstream::print!("{}", report::text::render_lint(&report)),
        Format::Json => print!("{}", report::json::render_lint(&report)?),
        Format::Junit => print!("{}", report::junit::render_lint(&report)?),
    }
    Ok(ExitCode::from(report.exit_code()))
}

fn init(
    cli: &Cli,
    schemas: &[String],
    output: &Path,
    force: bool,
    preset: Option<Preset>,
) -> Result<ExitCode> {
    if !force && output.exists() {
        return Err(InitError::Exists(output.to_path_buf()).into());
    }
    let Ok(url) = env::var("DATABASE_URL") else {
        bail!("`DATABASE_URL` is not set; init reads the database to scaffold the spec from it");
    };
    let target = guard(cli, &url, &[])?;
    let scaffold = init::introspect(&target, schemas, preset)?;
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

fn preset(name: &str) -> Result<Preset, String> {
    Preset::from_name(name).ok_or_else(|| {
        let known: Vec<_> = Preset::ALL.iter().map(|p| p.name()).collect();
        format!("expected one of: {}", known.join(", "))
    })
}

fn load(cli: &Cli) -> Result<(Config, Source, Target)> {
    let (config, source) = config::load(&cli.config)?;
    let target = guard(cli, &config.database.url, &config.safety.allowed_hosts)?;
    Ok((config, source, target))
}

/// Parses `url` and applies the safety guard, printing a warning for each override it relied on.
fn guard(cli: &Cli, url: &str, allowed_hosts: &[String]) -> Result<Target> {
    let allow = safety::Allow {
        remote: cli.allow_remote,
        insecure: cli.allow_insecure,
    };
    let (target, warnings) = safety::check(Target::parse(url)?, allowed_hosts, allow)?;
    let (warning, emphasis) = (style::WARNING, style::EMPHASIS);
    for text in warnings {
        anstream::eprintln!("{warning}warning{warning:#}{emphasis}:{emphasis:#} {text}");
    }
    Ok(target)
}

fn located(err: &anyhow::Error) -> Option<&ConfigError> {
    let invalid = match err.downcast_ref::<RunError>() {
        Some(RunError::Config(invalid))
        | Some(RunError::Pg(PgError::SetupTransactionControl(invalid))) => invalid,
        _ => err.downcast_ref::<ConfigError>()?,
    };
    matches!(invalid, ConfigError::Invalid { .. }).then_some(invalid)
}
