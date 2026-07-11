//! Headless CLI sidecar entrypoint. This binary intentionally keeps a real
//! console so stdin/stdout remain available for the NDJSON protocol.

use std::io::{self, Write};
use std::process::ExitCode;

use nte_dps_tool::api::dto::DevicesResult;
use nte_dps_tool::api::response::VersionResult;
use nte_dps_tool::cli::args::{self, Command};
use nte_dps_tool::core::capture;

fn main() -> ExitCode {
    let command = match args::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };

    match command {
        Command::Serve(options) => ExitCode::from(nte_dps_tool::cli::stdio::serve(options) as u8),
        Command::Version => write_json(&VersionResult::default()),
        Command::Devices => match capture::enumerate_devices() {
            Ok(devices) => write_json(&DevicesResult::new(&devices)),
            Err(_) => {
                eprintln!("error: Npcap is unavailable or device enumeration failed");
                ExitCode::from(1)
            }
        },
    }
}

fn write_json(value: &impl serde::Serialize) -> ExitCode {
    let mut stdout = io::stdout().lock();
    if serde_json::to_writer(&mut stdout, value).is_err()
        || stdout.write_all(b"\n").is_err()
        || stdout.flush().is_err()
    {
        eprintln!("error: failed to write JSON output");
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
