use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use mujina::{apply_patches, parse_with_option, restore_backup};

#[derive(Parser, Debug)]
#[command(name = "cargo-mujina", about = "Cargo patch manager tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Update Cargo.toml and run cargo bench
    Bench(WithArgs),
    /// Update Cargo.toml and run cargo build
    #[command(visible_alias = "b")]
    Build(WithArgs),
    /// Update Cargo.toml and run cargo check
    #[command(visible_alias = "c")]
    Check(WithArgs),
    /// Update Cargo.toml and run cargo clippy
    Clippy(WithArgs),
    /// Update Cargo.toml and run cargo doc
    #[command(visible_alias = "d")]
    Doc(WithArgs),
    /// Update Cargo.toml and run cargo fetch
    Fetch(WithArgs),
    /// Update Cargo.toml and run cargo metadata
    Metadata(WithArgs),
    /// Update Cargo.toml and run cargo miri
    Miri(WithArgs),
    /// Update Cargo.toml and run cargo run
    #[command(visible_alias = "r")]
    Run(WithArgs),
    /// Update Cargo.toml and run cargo rustc
    Rustc(WithArgs),
    /// Update Cargo.toml and run cargo rustdoc
    Rustdoc(WithArgs),
    /// Update Cargo.toml and run cargo test
    #[command(visible_alias = "t")]
    Test(WithArgs),
    /// Update Cargo.toml and run cargo tree
    Tree(WithArgs),
    /// Update Cargo.toml and run cargo vendor
    Vendor(WithArgs),
    /// Update Cargo.toml and exit (does not invoke cargo)
    Edit(WithArgs),
    /// Restore Cargo.toml from Cargo.toml.org
    Restore(WithArgs),
}

impl Commands {
    /// The `cargo` subcommand to run afterward, or `None` if this variant should not invoke
    /// cargo at all (i.e. `Edit`).
    fn cargo_subcommand(&self) -> Option<&'static str> {
        match self {
            Commands::Bench(_) => Some("bench"),
            Commands::Build(_) => Some("build"),
            Commands::Check(_) => Some("check"),
            Commands::Clippy(_) => Some("clippy"),
            Commands::Doc(_) => Some("doc"),
            Commands::Fetch(_) => Some("fetch"),
            Commands::Metadata(_) => Some("metadata"),
            Commands::Miri(_) => Some("miri"),
            Commands::Run(_) => Some("run"),
            Commands::Rustc(_) => Some("rustc"),
            Commands::Rustdoc(_) => Some("rustdoc"),
            Commands::Test(_) => Some("test"),
            Commands::Tree(_) => Some("tree"),
            Commands::Vendor(_) => Some("vendor"),
            Commands::Edit(_) => None,
            Commands::Restore(_) => None,
        }
    }

    fn with_args(&self) -> &WithArgs {
        match self {
            Commands::Bench(args) |
            Commands::Build(args) |
            Commands::Check(args) |
            Commands::Clippy(args) |
            Commands::Doc(args) |
            Commands::Fetch(args) |
            Commands::Metadata(args) |
            Commands::Miri(args) |
            Commands::Run(args) |
            Commands::Rustc(args) |
            Commands::Rustdoc(args) |
            Commands::Test(args) |
            Commands::Tree(args) |
            Commands::Vendor(args) |
            Commands::Edit(args) |
            Commands::Restore(args) => args,
        }
    }
}

#[derive(Args, Debug)]
struct WithArgs {
    /// Patch options to specify in [workspace.dependencies] (e.g. { path = "..." } or
    /// { git = "...", branch = "..." }).
    /// The target that path / git(+branch) points to must be a project that has a
    /// workspace-configured Cargo.toml directly under it.
    #[arg(short = 'W', long = "with", number_of_values = 1, action = clap::ArgAction::Append)]
    with: Vec<String>,

    /// Arguments to pass through to the cargo subcommand (build / test / run).
    /// These are appended directly after `cargo <subcommand>`, so cargo-level flags work as-is
    /// (e.g. `mujina build -- --release` -> `cargo build --release`). For `test` / `run`, if you
    /// want to forward arguments to the test binary / the running binary itself rather than to
    /// cargo, include cargo's own `--` separator explicitly, e.g.
    /// `mujina test -- -- --nocapture` -> `cargo test -- --nocapture`.
    #[arg(last = true)]
    cargo_args: Vec<String>,
}

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().collect();

    if args.get(1).map(|s| s.as_str()) == Some("mujina") {
        args.remove(1);
    }
    let cli = Cli::parse_from(args);

    let current_dir = std::env::current_dir().context("Failed to get the current directory")?;

    // For the Restore command, perform restoration only without applying patches and exit
    if matches!(cli.command, Commands::Restore(_)) {
        if restore_backup(&current_dir)? {
            println!("Successfully restored Cargo.toml from Cargo.toml.org");
        } else {
            println!("No Cargo.toml.org found. Cargo.toml is already in its original state.");
        }
        return Ok(());
    }

    let with_args = cli.command.with_args();
    let with_specs = with_args
        .with
        .iter()
        .map(|s| parse_with_option(s))
        .collect::<Result<Vec<_>>>()?;

    apply_patches(&current_dir, &with_specs)?;

    if let Some(subcommand) = cli.command.cargo_subcommand() {
        run_cargo_subcommand(subcommand, &with_args.cargo_args)?;
    }

    Ok(())
}

/// Runs `cargo <subcommand> <cargo_args...>`, passing arguments through unmodified, and exits
/// with the same status code cargo returned if it failed.
fn run_cargo_subcommand(subcommand: &str, cargo_args: &[String]) -> Result<()> {
        let mut cmd = std::process::Command::new("cargo");
    cmd.arg(subcommand);
    cmd.args(cargo_args);

    let status = cmd
        .status()
        .with_context(|| format!("Failed to run cargo {}", subcommand))?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
