use std::error::Error;

#[derive(Debug, Eq, PartialEq)]
enum Command {
    List,
    Get { index: usize },
    Set { percent: u32, index: usize },
    Change { delta: i32, index: usize },
    Capabilities { index: usize },
    Hdr,
    ToggleHdr,
    Help,
    Version,
}

pub(crate) fn run(args: impl IntoIterator<Item = String>) -> Result<(), Box<dyn Error>> {
    match parse(args)? {
        Command::List => {
            for monitor in brightwheel::list()? {
                let role = if monitor.primary {
                    "primary"
                } else {
                    "secondary"
                };
                match monitor.brightness {
                    Ok(value) => println!(
                        "{}\t{}\t{}\t{}/{}\t{}%",
                        monitor.index,
                        role,
                        monitor.description,
                        value.current,
                        value.maximum,
                        value.percent()
                    ),
                    Err(error) => println!(
                        "{}\t{}\t{}\tunsupported\t{}",
                        monitor.index, role, monitor.description, error
                    ),
                }
            }
        }
        Command::Get { index } => println!("{}", brightwheel::get(index)?.percent()),
        Command::Set { percent, index } => {
            println!("{}", brightwheel::set(index, percent)?.percent());
        }
        Command::Change { delta, index } => {
            println!("{}", brightwheel::change(index, delta)?.percent());
        }
        Command::Capabilities { index } => println!("{}", brightwheel::capabilities(index)?),
        Command::Hdr => print_hdr_state(brightwheel::hdr::state()?.enabled),
        Command::ToggleHdr => print_hdr_state(brightwheel::hdr::toggle()?.enabled),
        Command::Help => println!("{USAGE}"),
        Command::Version => println!("bright {}", env!("CARGO_PKG_VERSION")),
    }
    Ok(())
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let command = args.next().unwrap_or_else(|| "help".to_owned());

    match command.as_str() {
        "list" => {
            ensure_no_extra(args)?;
            Ok(Command::List)
        }
        "get" => {
            let index = optional_index(&mut args)?;
            ensure_no_extra(args)?;
            Ok(Command::Get { index })
        }
        "set" => {
            let percent = required_u32(&mut args, "PERCENT")?;
            let index = optional_index(&mut args)?;
            ensure_no_extra(args)?;
            Ok(Command::Set { percent, index })
        }
        "change" => {
            let delta = required_i32(&mut args, "DELTA")?;
            let index = optional_index(&mut args)?;
            ensure_no_extra(args)?;
            Ok(Command::Change { delta, index })
        }
        "capabilities" | "caps" => {
            let index = optional_index(&mut args)?;
            ensure_no_extra(args)?;
            Ok(Command::Capabilities { index })
        }
        "hdr" => {
            ensure_no_extra(args)?;
            Ok(Command::Hdr)
        }
        "hdr-toggle" => {
            ensure_no_extra(args)?;
            Ok(Command::ToggleHdr)
        }
        "help" | "--help" | "-h" => Ok(Command::Help),
        "version" | "--version" | "-V" => Ok(Command::Version),
        _ => Err(format!("unknown command '{command}'\n\n{USAGE}")),
    }
}

fn optional_index(args: &mut impl Iterator<Item = String>) -> Result<usize, String> {
    args.next().map_or(Ok(0), |value| {
        value
            .parse()
            .map_err(|_| format!("invalid monitor index '{value}'"))
    })
}

fn required_u32(args: &mut impl Iterator<Item = String>, name: &str) -> Result<u32, String> {
    parse_required(args, name)
}

fn required_i32(args: &mut impl Iterator<Item = String>, name: &str) -> Result<i32, String> {
    parse_required(args, name)
}

fn parse_required<T>(args: &mut impl Iterator<Item = String>, name: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    let value = args.next().ok_or_else(|| format!("missing {name}"))?;
    value
        .parse()
        .map_err(|_| format!("invalid {name} '{value}'"))
}

fn ensure_no_extra(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    args.next().map_or(Ok(()), |value| {
        Err(format!("unexpected argument '{value}'"))
    })
}

fn print_hdr_state(enabled: bool) {
    println!("{}", if enabled { "on" } else { "off" });
}

const USAGE: &str = "Usage:
  bright list
  bright get [MONITOR]
  bright set PERCENT [MONITOR]
  bright change DELTA [MONITOR]
  bright capabilities [MONITOR]
  bright hdr
  bright hdr-toggle

PERCENT is 0..100. MONITOR is an optional numeric index shown by bright list.
Type the number without square brackets; the default is 0.";

#[cfg(test)]
mod tests {
    use super::{Command, USAGE, parse};

    fn args<'a>(values: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
        values.iter().map(ToString::to_string)
    }

    #[test]
    fn defaults_to_help() {
        assert_eq!(parse(args(&[])), Ok(Command::Help));
    }

    #[test]
    fn parses_commands_and_default_monitor() {
        assert_eq!(parse(args(&["list"])), Ok(Command::List));
        assert_eq!(parse(args(&["get"])), Ok(Command::Get { index: 0 }));
        assert_eq!(
            parse(args(&["set", "77"])),
            Ok(Command::Set {
                percent: 77,
                index: 0
            })
        );
        assert_eq!(
            parse(args(&["change", "-5"])),
            Ok(Command::Change {
                delta: -5,
                index: 0
            })
        );
    }

    #[test]
    fn parses_explicit_monitor_indexes_and_aliases() {
        assert_eq!(parse(args(&["get", "2"])), Ok(Command::Get { index: 2 }));
        assert_eq!(
            parse(args(&["caps", "3"])),
            Ok(Command::Capabilities { index: 3 })
        );
        assert_eq!(parse(args(&["hdr"])), Ok(Command::Hdr));
        assert_eq!(parse(args(&["hdr-toggle"])), Ok(Command::ToggleHdr));
    }

    #[test]
    fn parses_help_and_version_aliases() {
        for alias in ["help", "--help", "-h"] {
            assert_eq!(parse(args(&[alias])), Ok(Command::Help));
        }
        for alias in ["version", "--version", "-V"] {
            assert_eq!(parse(args(&[alias])), Ok(Command::Version));
        }
    }

    #[test]
    fn reports_missing_and_invalid_values() {
        assert_eq!(parse(args(&["set"])), Err("missing PERCENT".to_owned()));
        assert_eq!(
            parse(args(&["set", "bright"])),
            Err("invalid PERCENT 'bright'".to_owned())
        );
        assert_eq!(
            parse(args(&["change", "up"])),
            Err("invalid DELTA 'up'".to_owned())
        );
        assert_eq!(
            parse(args(&["get", "primary"])),
            Err("invalid monitor index 'primary'".to_owned())
        );
    }

    #[test]
    fn rejects_unexpected_arguments() {
        assert_eq!(
            parse(args(&["list", "1"])),
            Err("unexpected argument '1'".to_owned())
        );
        assert_eq!(
            parse(args(&["set", "50", "0", "extra"])),
            Err("unexpected argument 'extra'".to_owned())
        );
    }

    #[test]
    fn unknown_command_includes_usage() {
        assert_eq!(
            parse(args(&["wat"])),
            Err(format!("unknown command 'wat'\n\n{USAGE}"))
        );
    }
}
