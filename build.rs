use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/brightwheel.ico");
    println!("cargo:rerun-if-changed=assets/brightwheel.rc");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("brightwheel.res");
    let compiler = find_resource_compiler().unwrap_or_else(|| {
        panic!("Windows SDK resource compiler rc.exe was not found");
    });

    let status = Command::new(&compiler)
        .current_dir(manifest_dir.join("assets"))
        .arg("/nologo")
        .arg("/fo")
        .arg(&output)
        .arg("brightwheel.rc")
        .status()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", compiler.display()));
    assert!(status.success(), "rc.exe failed with status {status}");

    println!("cargo:rustc-link-arg-bin=brightwheel={}", output.display());
}

fn find_resource_compiler() -> Option<PathBuf> {
    if let Some(path) = env::var_os("WindowsSdkVerBinPath") {
        let candidate = PathBuf::from(path).join(host_arch()).join("rc.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let program_files = env::var_os("ProgramFiles(x86)")?;
    let root = PathBuf::from(program_files).join(r"Windows Kits\10\bin");
    newest_sdk_compiler(&root)
}

fn newest_sdk_compiler(root: &Path) -> Option<PathBuf> {
    let mut versions: Vec<PathBuf> = fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    versions.sort_unstable_by(|left, right| right.file_name().cmp(&left.file_name()));
    versions
        .into_iter()
        .map(|version| version.join(host_arch()).join("rc.exe"))
        .find(|candidate| candidate.is_file())
}

fn host_arch() -> &'static str {
    match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    }
}
