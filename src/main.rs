//! Command-line companion for `BrightWheel` monitor and HDR controls.

#![cfg_attr(not(windows), allow(dead_code, unused_imports))]

#[cfg(not(windows))]
compile_error!("bright only supports Windows");

#[cfg(windows)]
mod cli;

#[cfg(windows)]
use std::{env, process};

#[cfg(windows)]
fn main() {
    if let Err(error) = cli::run(env::args().skip(1)) {
        eprintln!("bright: {error}");
        process::exit(1);
    }
}
