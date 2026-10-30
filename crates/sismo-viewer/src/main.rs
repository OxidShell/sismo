//! `sismo`: desktop viewer and headless CLI for sismo recordings and live apps.

mod cli;
mod client;
#[cfg(feature = "gui")]
mod gui;
mod workspace;

use clap::Parser;

fn main() -> std::process::ExitCode {
    let cli = cli::Cli::parse();
    match cli.command {
        None => launch(None, None, None),
        Some(cli::Command::Open { file, compare }) => launch(Some(file), None, compare),
        Some(cli::Command::Connect { target }) => match cli::resolve(&target) {
            Ok(addr) => launch(None, Some(addr), None),
            Err(error) => {
                eprintln!("error: {error}");
                std::process::ExitCode::FAILURE
            }
        },
        Some(command) => match cli::run(command) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                std::process::ExitCode::FAILURE
            }
        },
    }
}

#[cfg(feature = "gui")]
fn launch(
    file: Option<std::path::PathBuf>,
    live: Option<std::net::SocketAddr>,
    compare: Option<std::path::PathBuf>,
) -> std::process::ExitCode {
    gui::run(file, live, compare.map(gui::CompareWith));
    std::process::ExitCode::SUCCESS
}

#[cfg(not(feature = "gui"))]
fn launch(
    _file: Option<std::path::PathBuf>,
    _live: Option<std::net::SocketAddr>,
    _compare: Option<std::path::PathBuf>,
) -> std::process::ExitCode {
    eprintln!("this build has no viewer; run `sismo --help` for the CLI");
    std::process::ExitCode::FAILURE
}
