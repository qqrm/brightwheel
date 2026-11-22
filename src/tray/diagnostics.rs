use std::fs::{self, OpenOptions};
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 256 * 1024;

pub(crate) fn log(message: &str) {
    // Diagnostics must never prevent starting, recovering, or exiting the app.
    let _ = write_log(message);
}

fn write_log(message: &str) -> std::io::Result<()> {
    let Some(directory) = std::env::var_os("LOCALAPPDATA") else {
        return Ok(());
    };
    let directory = std::path::PathBuf::from(directory).join("BrightWheel");
    fs::create_dir_all(&directory)?;
    let path = directory.join("runtime.log");
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES) {
        let previous = directory.join("runtime.previous.log");
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(&path, previous);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{timestamp} pid={} {message}", std::process::id())
}
