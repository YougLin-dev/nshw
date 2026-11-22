//! The selector's administration commands. NSH's own commands and package
//! verification never pass through this module.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, bail};

use crate::{cache, exec, manifest, release, trust};

/// `nshw toolchain ...`, never NSH's own commands or package verification.
pub(crate) fn own_command(args: &[OsString]) -> anyhow::Result<()> {
    let command = args.first().and_then(|arg| arg.to_str()).unwrap_or("help");
    match command {
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        "version" | "-V" | "--version" => {
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
        other => bail!("unknown command {other}; try nshw toolchain help"),
    }
}

fn pin_command(args: &[OsString]) -> anyhow::Result<()> {
    let Some(version) = args.first().and_then(|v| v.to_str()) else {
        bail!("usage: nshw toolchain pin <release|rev=FULL_GIT_SHA> [app]")
    };
    // Reuse the very same parser used by execution, so pin never writes a
    // value that the launcher will later reject.
    let example = serde_json::json!({"toolchain":{"nsh":{"version":version}}});
    let config: manifest::Config = serde_json::from_value(example)?;
    manifest::pin(&config)?.context("invalid pin")?;
    let app = app_for_pin(args.get(1).map(Path::new))?;
    let path = app.join("nsh.json");
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
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
        .or_insert_with(|| serde_json::json!({"repository": release::DEFAULT_REPOSITORY}));
    let nsh = nsh
        .as_object_mut()
        .context("toolchain.nsh must be an object")?;
    nsh.insert(
        "version".into(),
        serde_json::Value::String(version.to_owned()),
    );
    fs::write(&path, serde_json::to_string_pretty(&value)? + "\n")?;
    println!("pinned NSH {version} in {}", path.display());
    Ok(())
}

/// `pin` accepts an app directory or its `nsh.json`; an unrelated path is a
/// mistake, not a reason to silently pin the current directory.
fn app_for_pin(explicit: Option<&Path>) -> anyhow::Result<PathBuf> {
    match explicit {
        None => manifest::find_app(&[]),
        Some(path) if path.is_dir() => {
            if path.join("nsh.json").is_file() {
                Ok(path.to_owned())
            } else {
                bail!("{} has no nsh.json", path.display())
            }
        }
        Some(path) if path.file_name().is_some_and(|name| name == "nsh.json") && path.is_file() => {
            path.parent()
                .map(Path::to_owned)
                .context("nsh.json has no parent directory")
        }
        Some(path) => bail!("{} is not an app directory or nsh.json", path.display()),
    }
}

fn status(app: &Path, config: &manifest::Config, pin: &manifest::Pin) -> anyhow::Result<()> {
    let expected = manifest::expected_rev(app, config)?;
    if let Some(rev) = &pin.revision
        && let Some(expected) = expected.as_deref()
        && expected != rev
    {
        bail!("the toolchain pins {rev}, but Cargo.lock has nsh-core at {expected}");
    }
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
        expected.as_deref().unwrap_or("not pinned (no Rust crate)")
    );
    Ok(())
}

fn verify(app: &Path, config: &manifest::Config, pin: &manifest::Pin) -> anyhow::Result<()> {
    let expected = manifest::expected_rev(app, config)?;
    if let Some(rev) = pin.revision.as_deref() {
        if let Some(expected) = expected.as_deref()
            && expected != rev
        {
            bail!("the toolchain pins {rev}, but Cargo.lock has nsh-core at {expected}");
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
                bail!(
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
                bail!(
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
        (Some(rev), None) => bail!(
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

pub(crate) fn print_help() {
    println!(
        "\
nshw {}

Runs the nsh CLI a project pins in nsh.json.

Usage:
  nshw [nsh arguments...]               transparent NSH execution
  nshw -- [nsh arguments...]            explicit NSH pass-through
  nshw toolchain status [app]           show the pin, cache and selected CLI
  nshw toolchain verify [app]           check the pin against the release and Cargo.lock
  nshw toolchain pin <version|rev=SHA>  write the exact toolchain pin in nsh.json
  nshw toolchain prefetch [app]         obtain the pinned CLI now
  nshw toolchain which [app]            print the selected CLI path
  nshw toolchain help                   show this help
  nshw toolchain version                show the selector's version

The pin, in nsh.json:
  \"toolchain\": {{
    \"nsh\": {{ \"version\": \"0.6.1\", \"repository\": \"longbridge/nsh\" }}
  }}

`version` is an exact release version (no ranges or `latest`), or
`rev=<40-hex-Git-commit>`. `repository` defaults to longbridge/nsh. Releases
are downloaded and their SHA-256 is verified. Git pins are built once from
source with Cargo. Toolchains are cached under `$NSH_CACHE` (or the platform
cache directory). Non-default repositories need an explicit trust ack through
`NSH_TOOLCHAIN_ALLOW` or an interactive confirmation.

Outside an app, nshw runs the installed nsh: `NSH_FALLBACK`, the standard
distribution, then `nsh` on PATH. nshw never rewrites application manifests
or lockfiles.

Install nshw as `nsh` ahead of a distribution's `bin`, or keep the
distribution's `nsh` and run `nshw` explicitly.",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> PathBuf {
        let app = std::env::temp_dir().join(format!(
            "nshw-pin-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&app);
        fs::create_dir_all(&app).unwrap();
        fs::write(
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
            serde_json::from_str(&fs::read_to_string(app.join("nsh.json")).unwrap()).unwrap();
        assert_eq!(value["toolchain"]["nsh"]["version"], "0.6.1");
        assert_eq!(value["name"], "fixture");
        assert_eq!(value["capabilities"]["network"]["hosts"][0], "example.com");
        let _ = fs::remove_dir_all(&app);
    }

    #[test]
    fn invalid_pins_are_rejected_without_modifying_the_file() {
        let app = fixture();
        let before = fs::read_to_string(app.join("nsh.json")).unwrap();
        let error = pin_command(&[
            OsString::from("latest"),
            app.join("nsh.json").into_os_string(),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("must name a release"), "{error}");
        assert_eq!(before, fs::read_to_string(app.join("nsh.json")).unwrap());
        let _ = fs::remove_dir_all(&app);
    }

    #[test]
    fn pin_rejects_an_unrelated_explicit_path() {
        let app = fixture();
        let unrelated = app.join("notes.txt");
        fs::write(&unrelated, "not an app").unwrap();
        let error =
            pin_command(&[OsString::from("0.6.1"), unrelated.into_os_string()]).unwrap_err();
        assert!(
            error.to_string().contains("not an app directory"),
            "{error}"
        );
        let _ = fs::remove_dir_all(&app);
    }
}
