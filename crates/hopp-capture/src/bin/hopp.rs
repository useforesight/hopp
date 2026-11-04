//! `hopp`: Hopp from the terminal. Reads and writes the captures on this computer.

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "hopp", about = "Hopp from the terminal")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(flatten)]
    App(hopp_capture::cli::AppCommand),
    /// Transcripts of your calls, saved only on this computer.
    Capture {
        #[command(subcommand)]
        command: hopp_capture::commands::CaptureCommand,
    },
    /// Serve captured calls to coding agents over MCP (stdin/stdout).
    Mcp,
}

/// Why a capture couldn't do something goes to stderr, not nowhere.
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

fn main() -> Result<()> {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    match Cli::parse().command {
        Command::App(command) => hopp_capture::cli::run(command),
        Command::Capture { command } => hopp_capture::commands::run(command),
        Command::Mcp => hopp_capture::mcp::serve(),
    }
}
