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
        None => launch(None, None),
        Some(cli::Command::Open { file }) => launch(Some(file), None),
        Some(cli::Command::Connect { target }) => match cli::resolve(&target) {
            Ok(addr) => launch(None, Some(addr)),
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
) -> std::process::ExitCode {
    gui::run(file, live);
    std::process::ExitCode::SUCCESS
}

#[cfg(not(feature = "gui"))]
fn launch(
    _file: Option<std::path::PathBuf>,
    _live: Option<std::net::SocketAddr>,
) -> std::process::ExitCode {
    eprintln!("this build has no viewer; run `sismo --help` for the CLI");
    std::process::ExitCode::FAILURE
}
