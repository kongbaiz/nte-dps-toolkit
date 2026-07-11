use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve(ServeOptions),
    Version,
    Devices,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ServeOptions {
    pub data_dir: PathBuf,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let args: Vec<OsString> = args.into_iter().collect();
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        return Err(usage());
    };
    match command {
        "version" if args.get(1).is_some_and(|arg| arg == "--json") && args.len() == 2 => {
            Ok(Command::Version)
        }
        "devices" if args.get(1).is_some_and(|arg| arg == "--json") && args.len() == 2 => {
            Ok(Command::Devices)
        }
        "serve" => parse_serve(&args[1..]),
        _ => Err(usage()),
    }
}

fn parse_serve(args: &[OsString]) -> Result<Command, String> {
    let mut stdio = false;
    let mut data_dir = None;
    let mut log_level = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].to_str() {
            Some("--stdio") if !stdio => stdio = true,
            Some("--data-dir") if data_dir.is_none() => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "error: --data-dir requires a value".to_owned())?;
                if value.is_empty() || value.to_str().is_some_and(|value| value.starts_with("--")) {
                    return Err("error: --data-dir requires a non-empty value".to_owned());
                }
                data_dir = Some(PathBuf::from(value));
            }
            Some("--log-level") if !log_level => {
                index += 1;
                let value = args
                    .get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| "error: --log-level requires a UTF-8 value".to_owned())?;
                if value.is_empty() || value.starts_with("--") {
                    return Err("error: --log-level requires a non-empty value".to_owned());
                }
                log_level = true;
            }
            _ => return Err(usage()),
        }
        index += 1;
    }
    if !stdio {
        return Err("error: serve requires --stdio".to_owned());
    }
    Ok(Command::Serve(ServeOptions {
        data_dir: data_dir.unwrap_or_else(|| PathBuf::from("logs")),
    }))
}

fn usage() -> String {
    "error: unsupported arguments; usage: nte-core serve --stdio [--data-dir <path>] [--log-level <level>] | version --json | devices --json".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_all_commands() {
        assert_eq!(parse(args(&["version", "--json"])), Ok(Command::Version));
        assert_eq!(parse(args(&["devices", "--json"])), Ok(Command::Devices));
        assert_eq!(
            parse(args(&[
                "serve",
                "--data-dir",
                "capture-data",
                "--stdio",
                "--log-level",
                "info",
            ])),
            Ok(Command::Serve(ServeOptions {
                data_dir: PathBuf::from("capture-data")
            }))
        );
        assert_eq!(
            parse(args(&["serve", "--stdio"])),
            Ok(Command::Serve(ServeOptions {
                data_dir: PathBuf::from("logs")
            }))
        );
    }

    #[test]
    fn serve_requires_stdio_and_option_values() {
        assert_eq!(
            parse(args(&["serve"])).unwrap_err(),
            "error: serve requires --stdio"
        );
        assert_eq!(
            parse(args(&["serve", "--stdio", "--data-dir"])).unwrap_err(),
            "error: --data-dir requires a value"
        );
    }

    #[test]
    fn rejects_unknown_and_duplicate_options() {
        assert!(parse(args(&["unknown"])).is_err());
        assert!(parse(args(&["serve", "--stdio", "--stdio"])).is_err());
    }
}
