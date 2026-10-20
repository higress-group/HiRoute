use std::path::PathBuf;

use clap::{Parser, Subcommand};
use hiroute_release_facts::{check, generate};

#[derive(Debug, Parser)]
#[command(name = "hiroute-release-facts")]
#[command(about = "Compile or check deterministic offline HiRoute ReleaseFacts artifacts")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Generate(Arguments),
    Check(Arguments),
    Feed {
        #[command(subcommand)]
        command: FeedCommand,
    },
}

#[derive(Debug, Subcommand)]
enum FeedCommand {
    Build {
        #[arg(long)]
        check: bool,
        /// Exact PR base (or previous main revision) for immutable publication history checks.
        #[arg(long)]
        base: Option<String>,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

#[derive(Clone, Debug, clap::Args)]
struct Arguments {
    #[arg(long)]
    input: PathBuf,
    #[arg(long = "output-dir")]
    output_dir: PathBuf,
}

fn main() {
    let cli = Cli::parse();
    if let Command::Feed {
        command: FeedCommand::Build { check, root, base },
    } = &cli.command
    {
        if let Err(error) =
            hiroute_release_facts::feed::build_against(root, *check, base.as_deref())
        {
            eprintln!("hiroute-release-facts: {error}");
            std::process::exit(1);
        }
        return;
    }
    let result = match cli.command {
        Command::Generate(arguments) => generate(&arguments.input, &arguments.output_dir),
        Command::Check(arguments) => check(&arguments.input, &arguments.output_dir),
        Command::Feed { .. } => unreachable!(),
    };
    match result {
        Ok(compiled) => {
            for (name, digest) in compiled.digests() {
                println!("{name}\t{digest}");
            }
        }
        Err(error) => {
            eprintln!("hiroute-release-facts: {error}");
            std::process::exit(1);
        }
    }
}
