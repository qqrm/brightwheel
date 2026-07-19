#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

#[cfg(not(windows))]
compile_error!("bright only supports Windows");

#[cfg(windows)]
use std::{env, process};

#[cfg(windows)]
fn main() {
    if let Err(error) = run() {
        eprintln!("bright: {error}");
        process::exit(1);
    }
}

#[cfg(windows)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".to_owned());

    match command.as_str() {
        "list" => {
            ensure_no_extra(args)?;
            for monitor in brightwheel::list()? {
                match monitor.brightness {
                    Ok(value) => println!(
                        "{}\t{}\t{}\t{}/{}\t{}%",
                        monitor.index,
                        if monitor.primary {
                            "primary"
                        } else {
                            "secondary"
                        },
                        monitor.description,
                        value.current,
                        value.maximum,
                        value.percent()
                    ),
                    Err(error) => println!(
                        "{}\t{}\t{}\tunsupported\t{}",
                        monitor.index,
                        if monitor.primary {
                            "primary"
                        } else {
                            "secondary"
                        },
                        monitor.description,
                        error
                    ),
                }
            }
        }
        "get" => {
            let index = optional_index(&mut args)?;
            ensure_no_extra(args)?;
            println!("{}", brightwheel::get(index)?.percent());
        }
        "set" => {
            let percent = required_u32(&mut args, "PERCENT")?;
            let index = optional_index(&mut args)?;
            ensure_no_extra(args)?;
            println!("{}", brightwheel::set(index, percent)?.percent());
        }
        "change" => {
            let delta = required_i32(&mut args, "DELTA")?;
            let index = optional_index(&mut args)?;
            ensure_no_extra(args)?;
            println!("{}", brightwheel::change(index, delta)?.percent());
        }
        "capabilities" | "caps" => {
            let index = optional_index(&mut args)?;
            ensure_no_extra(args)?;
            println!("{}", brightwheel::capabilities(index)?);
        }
        "hdr" => {
            ensure_no_extra(args)?;
            let state = brightwheel::hdr::state()?;
            println!("{}", if state.enabled { "on" } else { "off" });
        }
        "hdr-toggle" => {
            ensure_no_extra(args)?;
            let state = brightwheel::hdr::toggle()?;
            println!("{}", if state.enabled { "on" } else { "off" });
        }
        "help" | "--help" | "-h" => print_usage(),
        "version" | "--version" | "-V" => println!("bright {}", env!("CARGO_PKG_VERSION")),
        _ => {
            return Err(format!("unknown command '{command}'\n\n{}", usage()).into());
        }
    }

    Ok(())
}

#[cfg(windows)]
fn optional_index(args: &mut impl Iterator<Item = String>) -> Result<usize, String> {
    match args.next() {
        Some(value) => value
            .parse()
            .map_err(|_| format!("invalid monitor index '{value}'")),
        None => Ok(0),
    }
}

#[cfg(windows)]
fn required_u32(args: &mut impl Iterator<Item = String>, name: &str) -> Result<u32, String> {
    let value = args.next().ok_or_else(|| format!("missing {name}"))?;
    value
        .parse()
        .map_err(|_| format!("invalid {name} '{value}'"))
}

#[cfg(windows)]
fn required_i32(args: &mut impl Iterator<Item = String>, name: &str) -> Result<i32, String> {
    let value = args.next().ok_or_else(|| format!("missing {name}"))?;
    value
        .parse()
        .map_err(|_| format!("invalid {name} '{value}'"))
}

#[cfg(windows)]
fn ensure_no_extra(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    match args.next() {
        Some(value) => Err(format!("unexpected argument '{value}'")),
        None => Ok(()),
    }
}

#[cfg(windows)]
fn print_usage() {
    println!("{}", usage());
}

#[cfg(windows)]
fn usage() -> &'static str {
    "Usage:
  bright list
  bright get [MONITOR]
  bright set PERCENT [MONITOR]
  bright change DELTA [MONITOR]
  bright capabilities [MONITOR]
  bright hdr
  bright hdr-toggle

PERCENT is 0..100. MONITOR defaults to 0."
}
