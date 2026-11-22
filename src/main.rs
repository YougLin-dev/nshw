//! `nshw` is transparent to NSH's own commands. Administration lives under
//! `nshw toolchain ...`; everything else reaches the selected CLI unchanged.
//!
//! Release pins install a checksum-verified prebuilt distribution. Git pins
//! build a distribution from exactly their full commit. Only the explicit
//! `toolchain pin` edits `nsh.json`; Cargo manifests are never rewritten.

mod archive;
mod cache;
mod command;
mod exec;
mod manifest;
mod release;
mod source;
#[cfg(test)]
mod test_support;
mod trust;

use std::{ffi::OsString, process::ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("nshw: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Only this explicit namespace belongs to the selector. In particular,
/// NSH's own `verify` verifies signed packages and is never intercepted.
fn run() -> anyhow::Result<()> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if !is_nsh_alias() && matches!(args.first().and_then(|a| a.to_str()), Some("toolchain")) {
        return command::own_command(&args[1..]);
    }
    if matches!(args.first().and_then(|a| a.to_str()), Some("--")) {
        return forward(&args[1..]);
    }
    if args.is_empty() && !is_nsh_alias() {
        command::print_help();
        return Ok(());
    }
    forward(&args)
}

fn is_nsh_alias() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy() == "nsh"))
        .unwrap_or(false)
}

fn forward(args: &[OsString]) -> anyhow::Result<()> {
    let Some(app) = manifest::locate_app(args)? else {
        return exec::fallback(args);
    };
    let config = manifest::read_config(&app)?;
    let Some(pin) = manifest::pin(&config)? else {
        return exec::fallback(args);
    };
    let bin = cache::ensure(&app, &config, &pin)?;
    exec::exec_pinned(&bin, args)
}

#[cfg(test)]
mod tests {
    #[test]
    fn installer_default_matches_package_version() {
        let script = include_str!("../install.sh");
        let expected = format!(
            "version=\"${{NSHW_VERSION:-{}}}\"",
            env!("CARGO_PKG_VERSION")
        );
        assert!(
            script.contains(&expected),
            "install.sh default must match Cargo.toml's version"
        );
    }
}
