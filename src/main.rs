use clap::{Parser, Subcommand};
use medl::inputs::{Inputs, ProcessEnv};
use medl::loader::FsLoader;
use medl::Medl;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "medl", version, about = "Resolve .medl configuration files")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Merge, check and resolve a .medl file and print the result as JSON
    Resolve {
        /// Entry .medl file
        file: PathBuf,
        /// Context value, repeatable: --ctx platform=mobile
        #[arg(long = "ctx", value_name = "KEY=VALUE")]
        ctx: Vec<String>,
        /// Secret value, repeatable: --secret API_KEY=abc
        #[arg(long = "secret", value_name = "KEY=VALUE")]
        secret: Vec<String>,
        /// Print lint warnings to stderr
        #[arg(long)]
        strict: bool,
    },
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let Command::Resolve {
        file,
        ctx,
        secret,
        strict,
    } = Cli::parse().command;

    let mut inputs = Inputs::new().with_env(ProcessEnv);
    for pair in &ctx {
        match parse_pair(pair) {
            Ok((k, v)) => inputs = inputs.with_ctx(&k, &v),
            Err(msg) => {
                eprintln!("error: --ctx {msg}");
                return 2;
            }
        }
    }
    let mut secrets = HashMap::new();
    for pair in &secret {
        match parse_pair(pair) {
            Ok((k, v)) => {
                secrets.insert(k, v);
            }
            Err(msg) => {
                eprintln!("error: --secret {msg}");
                return 2;
            }
        }
    }
    inputs = inputs.with_secret(secrets);

    let mut medl = Medl::new(Box::new(FsLoader));
    match medl.resolve(&file, &inputs) {
        Ok(output) => {
            if strict {
                for w in &output.warnings {
                    eprint!("{}", w.render(&medl.sources));
                }
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&output.value.to_json())
                    .expect("json serialization cannot fail")
            );
            0
        }
        Err(e) => {
            eprint!("{}", e.render(&medl.sources));
            1
        }
    }
}

fn parse_pair(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
        _ => Err(format!("expected KEY=VALUE, got `{s}`")),
    }
}
