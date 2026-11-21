//! `nshw` is transparent to NSH's own commands. Administration lives under
//! `nshw toolchain ...`; everything else reaches the selected CLI unchanged.
//!
//! Release pins install a checksum-verified prebuilt distribution. Git pins
//! build a distribution from exactly their full commit. Only the explicit
//! `toolchain pin` edits `nsh.json`; Cargo manifests are never rewritten.

mod archive;
mod cache;
mod exec;
mod manifest;
mod release;
mod source;
#[cfg(test)]
mod test_support;
mod trust;

use std::{ffi::OsString, path::Path, process::ExitCode};

use anyhow::Context as _;

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
        return own_command(&args[1..]);
    }
    if matches!(args.first().and_then(|a| a.to_str()), Some("--")) {
        return forward(&args[1..]);
    }
    if args.is_empty() && !is_nsh_alias() {
        print_help();
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
    let app = match manifest::find_app(args) {
        Ok(app) => app,
        Err(_) => return exec::fallback(args),
    };
    let config = manifest::read_config(&app)?;
    let Some(pin) = manifest::pin(&config)? else {
        return exec::fallback(args);
    };
    let bin = cache::ensure(&app, &config, &pin)?;
    exec::exec(&bin, args)
}

/// nshw's administration, never NSH's own commands or package verification.
fn own_command(args: &[OsString]) -> anyhow::Result<()> {
    let command = args.first().and_then(|arg| arg.to_str()).unwrap_or("help");
    match command {
        "help" => {
            print_help();
            Ok(())
        }
        "version" => {
            println!("nshw {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "pin" => pin_command(&args[1..]),
        "status" | "verify" | "prefetch" | "which" => {
            let app = manifest::find_app(&args[1..])?;
            let config = manifest::read_config(&app)?;
            let pin = manifest::pin(&config)?.with_context(|| {
                format!(
                    "{}: nsh.json has no toolchain.nsh pin",
                    app.join("nsh.json").display()
                )
            })?;
            match command {
                "status" => status(&app, &config, &pin),
                "verify" => verify(&app, &config, &pin),
                "prefetch" => {
                    println!("{}", cache::ensure(&app, &config, &pin)?.display());
                    Ok(())
                }
                _ => {
                    let host = release::host()?;
                    println!(
                        "{}",
                        cache::toolchain_dir(&pin, host)?
                            .join("bin")
                            .join(exec::exe("nsh"))
                            .display()
                    );
                    Ok(())
                }
            }
        }
        other => anyhow::bail!("unknown command {other}; try nshw toolchain help"),
    }
}

fn pin_command(args: &[OsString]) -> anyhow::Result<()> {
    let Some(version) = args.first().and_then(|v| v.to_str()) else {
        anyhow::bail!("usage: nshw toolchain pin <release|rev=FULL_GIT_SHA> [app]")
    };
    // Reuse the very same parser used by execution, so pin never writes a
    // value that the launcher will later reject.
    let example = serde_json::json!({"toolchain":{"nsh":{"version":version}}});
    let config: manifest::Config = serde_json::from_value(example)?;
    manifest::pin(&config)?.context("invalid pin")?;
    let app = match args.get(1).map(Path::new) {
        Some(path) if path.is_dir() => path.to_owned(),
        Some(path) if path.parent().is_some_and(|p| p.join("nsh.json").is_file()) => {
            path.parent().unwrap().to_owned()
        }
        _ => manifest::find_app(&[])?,
    };
    let path = app.join("nsh.json");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let mut value: serde_json::Value = serde_json::from_str(&text)?;
    let object = value
        .as_object_mut()
        .context("nsh.json must contain a JSON object")?;
    let toolchain = object
        .entry("toolchain")
        .or_insert_with(|| serde_json::json!({}));
    let toolchain = toolchain
        .as_object_mut()
        .context("toolchain must be an object")?;
    let nsh = toolchain
        .entry("nsh")
        .or_insert_with(|| serde_json::json!({"repository": crate::release::DEFAULT_REPOSITORY}));
    if !nsh.is_object() {
        anyhow::bail!("toolchain.nsh must be an object");
    }
    nsh.as_object_mut().unwrap().insert(
        "version".into(),
        serde_json::Value::String(version.to_owned()),
    );
    std::fs::write(&path, serde_json::to_string_pretty(&value)? + "\n")?;
    println!("pinned NSH {version} in {}", path.display());
    Ok(())
}

fn status(app: &Path, config: &manifest::Config, pin: &manifest::Pin) -> anyhow::Result<()> {
    let host = release::host()?;
    let dir = cache::toolchain_dir(pin, host)?;
    let bin = dir.join("bin").join(exec::exe("nsh"));
    println!("app: {}", app.display());
    println!("pin: {} ({})", pin.version, pin.repository);
    println!("tag: {}", pin.tag);
    println!("host: {host}");
    println!("toolchain: {}", dir.display());
    println!(
        "selected: {} ({})",
        bin.display(),
        if bin.is_file() { "present" } else { "missing" }
    );
    println!(
        "nsh-core: {}",
        manifest::expected_rev(app, config)?
            .as_deref()
            .unwrap_or("not pinned (no Rust crate)")
    );
    Ok(())
}

fn verify(app: &Path, config: &manifest::Config, pin: &manifest::Pin) -> anyhow::Result<()> {
    let expected = manifest::expected_rev(app, config)?;
    if let Some(rev) = pin.version.strip_prefix("rev=") {
        if let Some(expected) = expected.as_deref()
            && expected != rev
        {
            anyhow::bail!("the toolchain pins {rev}, but Cargo.lock has nsh-core at {expected}");
        }
        let bin = cache::toolchain_dir(pin, release::host()?)?
            .join("bin")
            .join(exec::exe("nsh"));
        if bin.is_file() {
            println!("ok: {} is {}", pin.version, bin.display());
        } else {
            println!(
                "ok: {rev} matches the project declaration; run nshw toolchain prefetch to build it"
            );
        }
        return Ok(());
    }
    trust::check(pin)?;
    let info = release::fetch(&pin.repository, &pin.tag)?;
    match (expected.as_deref(), info) {
        (Some(rev), Some(info)) => {
            if let Some(tag) = info.tag.as_deref()
                && tag != pin.tag
            {
                anyhow::bail!(
                    "{} describes tag {tag}, not {}",
                    release::RELEASE_INFO,
                    pin.tag
                );
            }
            let commit = info
                .commit
                .as_deref()
                .with_context(|| format!("{} has no commit in its nsh-release.json", pin.tag))?;
            if commit != rev {
                anyhow::bail!(
                    "{} is built from {commit}, but the app pins nsh-core at {rev}",
                    pin.tag
                );
            }
            println!(
                "ok: {} (nsh {}) is {rev}",
                pin.tag,
                info.version.as_deref().unwrap_or("unknown")
            );
        }
        (Some(rev), None) => anyhow::bail!(
            "{} has no nsh-release.json; cannot verify the pinned nsh-core {rev}",
            pin.tag
        ),
        (None, Some(_)) => println!("ok: {} (the app has no Rust crate to verify)", pin.tag),
        (None, None) => println!(
            "ok: {} (no Rust crate; the release has no nsh-release.json)",
            pin.tag
        ),
    }
    Ok(())
}

fn print_help() {
    println!(
        "\
nshw {}

Runs the nsh CLI a project pins in nsh.json.

Usage:
  nshw [nsh arguments...]          transparent NSH execution
  nshw -- [nsh arguments...]       explicit NSH pass-through
  nshw toolchain status [app]       show the pin, cache and selected CLI
  nshw toolchain verify [app]          check the pin against the release and Cargo.lock
  nshw toolchain pin <version|rev=SHA> write the exact toolchain pin in nsh.json
  nshw toolchain prefetch [app]        obtain the pinned CLI now
  nshw toolchain which [app]           print the selected CLI path
  nshw toolchain help
  nshw toolchain version

The pin, in nsh.json:
  \"toolchain\": {{
    \"nsh\": {{ \"version\": \"0.6.1\", \"repository\": \"longbridge/nsh\" }}
  }}

`version` is an exact release version (no ranges or `latest`), or
`rev=<40-hex-Git-commit>`. `repository` defaults to longbridge/nsh. Releases
are downloaded and their SHA-256 is verified. Git pins are built once from
source with Cargo. Toolchains are cached under `$NSH_CACHE` (or the platform
cache directory). Non-default repositories need an explicit trust ack through
`NSH_TOOLCHAIN_ALLOW` or an interactive confirmation. nshw never rewrites
application manifests or lockfiles.

Install nshw as `nsh` ahead of a distribution's `bin`, or keep the
distribution's `nsh` and run `nshw` explicitly.",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> std::path::PathBuf {
        let app = std::env::temp_dir().join(format!(
            "nshw-pin-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&app);
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(
            app.join("nsh.json"),
            serde_json::to_string(&json!({
                "name": "fixture",
                "capabilities": {"network": {"hosts": ["example.com"]}}
            }))
            .unwrap(),
        )
        .unwrap();
        app
    }

    #[test]
    fn pin_preserves_the_other_project_settings() {
        let app = fixture();
        pin_command(&[
            OsString::from("0.6.1"),
            app.join("nsh.json").into_os_string(),
        ])
        .unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(app.join("nsh.json")).unwrap()).unwrap();
        assert_eq!(value["toolchain"]["nsh"]["version"], "0.6.1");
        assert_eq!(value["name"], "fixture");
        assert_eq!(value["capabilities"]["network"]["hosts"][0], "example.com");
        let _ = std::fs::remove_dir_all(&app);
    }

    #[test]
    fn invalid_pins_are_rejected_without_modifying_the_file() {
        let app = fixture();
        let before = std::fs::read_to_string(app.join("nsh.json")).unwrap();
        let error = pin_command(&[
            OsString::from("latest"),
            app.join("nsh.json").into_os_string(),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("must name a release"), "{error}");
        assert_eq!(
            before,
            std::fs::read_to_string(app.join("nsh.json")).unwrap()
        );
        let _ = std::fs::remove_dir_all(&app);
    }
}
