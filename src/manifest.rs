//! The project side: finding `nsh.json`, reading its toolchain pin, and
//! checking the `nsh-core` revision an app's Rust crate resolves to.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, bail};
use serde::Deserialize;

/// The parts of `nsh.json` nshw reads.
#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    #[serde(default)]
    toolchain: Option<Toolchain>,
    #[serde(default)]
    rust: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Toolchain {
    #[serde(default)]
    nsh: Option<NshPin>,
}

#[derive(Debug, Deserialize)]
struct NshPin {
    version: String,
    #[serde(default)]
    repository: Option<String>,
}

/// The exact nsh release a project pins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pin {
    /// The version as `nsh.json` writes it, e.g. `0.6.1` or `v0.6.1`.
    pub(crate) version: String,
    /// The release tag the version names, e.g. `v0.6.1`.
    pub(crate) tag: String,
    /// The normalized full Git commit, absent for a release.
    pub(crate) revision: Option<String>,
    /// `owner/repo` whose GitHub releases hold the distribution.
    pub(crate) repository: String,
}

pub(crate) fn consumes_value(name: &str) -> bool {
    matches!(
        name,
        "--name"
            | "--template"
            | "--nsh-source"
            | "--out"
            | "--filter"
            | "--junit"
            | "--target"
            | "--browser"
            | "--web-host"
            | "--device"
            | "--display"
            | "--tsc"
            | "--port"
            | "--points-per-cell"
            | "--window-size"
            | "--id"
            | "--app-version"
            | "--build"
            | "--icon"
            | "--sign"
            | "--profile"
            | "--keystore"
            | "--key-alias"
            | "--keystore-password"
            | "--key_alias"
            | "--keystore_password"
            | "--abi"
            | "--format"
            | "--arch"
            | "--asset-url"
            | "--key"
            | "--cert"
            | "--source-map"
            | "--min-runtime"
            | "--package"
            | "--entry"
            | "--notes"
            | "--link"
            | "--base-url"
            | "--bundle"
            | "--root"
            | "--revocations"
            | "--extend"
            | "--sequence"
            | "--days"
            | "--apps"
    )
}

/// Only an app command's positional entry may select a project. NSH's argv
/// is forwarded verbatim, but options still have their own grammar: treating
/// every path-shaped token as an app can silently select another toolchain.
fn app_argument(args: &[OsString]) -> Option<OsString> {
    let mut index = 0;
    let mut positional = false;
    let mut command = None;
    while index < args.len() {
        let arg = args[index].to_string_lossy();
        if arg == "--" {
            positional = true;
            index += 1;
            continue;
        }
        if !positional && (arg == "-v" || arg == "--verbose" || arg.starts_with("--verbose=")) {
            index += 1;
            continue;
        }
        if !positional && arg.starts_with('-') {
            if !arg.contains('=') && consumes_value(&arg) {
                index += 1;
            }
            index += 1;
            continue;
        }
        if command.is_none() {
            command = Some(arg.into_owned());
            index += 1;
            continue;
        }
        let app_command = matches!(
            command.as_deref(),
            Some(
                "init"
                    | "build"
                    | "test"
                    | "check"
                    | "dev"
                    | "run"
                    | "package"
                    | "publish"
                    | "verify"
            )
        );
        return app_command.then(|| args[index].clone());
    }
    // No explicit entry: app commands take their current directory.
    matches!(
        command.as_deref(),
        Some(
            "init" | "build" | "test" | "check" | "dev" | "run" | "package" | "publish" | "verify",
        )
    )
    .then(|| OsString::from("."))
}

/// The explicit app entry wins; otherwise this directory or an ancestor.
pub(crate) fn find_app(args: &[OsString]) -> anyhow::Result<PathBuf> {
    if let Some(entry) = app_argument(args) {
        let path = Path::new(&entry);
        let start = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        if let Some(app) = start.ancestors().find(|dir| dir.join("nsh.json").is_file()) {
            return Ok(app.to_owned());
        }
        bail!(
            "{} is not inside an app containing nsh.json",
            path.display()
        );
    }
    let cwd = std::env::current_dir().context("current directory")?;
    if let Some(app) = [cwd.join("src"), cwd.clone()]
        .into_iter()
        .find(|dir| dir.join("nsh.json").is_file())
    {
        return Ok(app.canonicalize().unwrap_or(app));
    }
    if let Some(app) = cwd.ancestors().find(|dir| dir.join("nsh.json").is_file()) {
        return Ok(app.canonicalize().unwrap_or_else(|_| app.to_owned()));
    }
    bail!("no nsh.json found here or in the arguments; run inside an app or pass its directory")
}

/// `nsh.json` in `app`.
pub(crate) fn read_config(app: &Path) -> anyhow::Result<Config> {
    let path = app.join("nsh.json");
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} is not JSON", path.display()))
}

/// The toolchain pin, or `None` for an app that does not pin one.
pub(crate) fn pin(config: &Config) -> anyhow::Result<Option<Pin>> {
    let Some(toolchain) = &config.toolchain else {
        return Ok(None);
    };
    let Some(nsh) = &toolchain.nsh else {
        return Ok(None);
    };
    let version = nsh.version.trim();
    if version.is_empty() {
        bail!("\"toolchain.nsh.version\" is empty");
    }
    if let Some(rev) = version.strip_prefix("rev=") {
        if rev.len() != 40 || !rev.bytes().all(|c| c.is_ascii_hexdigit()) {
            bail!("\"toolchain.nsh.version\" rev must be a full 40-hex Git commit");
        }
        let rev = rev.to_ascii_lowercase();
        let repository = nsh
            .repository
            .as_deref()
            .unwrap_or(super::release::DEFAULT_REPOSITORY)
            .to_owned();
        validate_repository(&repository)?;
        return Ok(Some(Pin {
            version: format!("rev={rev}"),
            tag: format!("rev={rev}"),
            revision: Some(rev),
            repository,
        }));
    }
    if version.chars().any(|c| {
        c.is_whitespace()
            || matches!(
                c,
                '/' | '\\' | '*' | '^' | '~' | '>' | '<' | '=' | ',' | '|' | ':' | '?' | '#' | '%'
            )
    }) || version.contains("..")
    {
        bail!("\"toolchain.nsh.version\" must be one exact version, not {version:?}");
    }
    let lowered = version.to_ascii_lowercase();
    if lowered == "latest" || lowered == "nightly" {
        bail!("\"toolchain.nsh.version\" must name a release, not {version:?}");
    }
    let tag = if version
        .strip_prefix('v')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
    {
        version.to_owned()
    } else {
        format!("v{version}")
    };
    let repository = nsh
        .repository
        .as_deref()
        .map(str::trim)
        .filter(|repo| !repo.is_empty())
        .unwrap_or(super::release::DEFAULT_REPOSITORY)
        .to_owned();
    validate_repository(&repository)?;
    Ok(Some(Pin {
        version: version.to_owned(),
        tag,
        revision: None,
        repository,
    }))
}

fn validate_repository(repository: &str) -> anyhow::Result<()> {
    let Some((owner, repo)) = repository.split_once('/') else {
        bail!(
            "\"toolchain.nsh.repository\" must be owner/repo, e.g. longbridge/nsh, not {repository:?}"
        );
    };
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        bail!("\"toolchain.nsh.repository\" must be owner/repo, not {repository:?}");
    }
    if repository
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, ':' | '@' | '\\' | '?' | '#' | '~'))
        || repository.contains("..")
    {
        bail!("\"toolchain.nsh.repository\" must be owner/repo, not {repository:?}");
    }
    Ok(())
}

/// The `nsh-core` revision the app's Rust crates pin, if any: every
/// `Cargo.lock` of its `rust` crates that takes nsh from Git must name one
/// revision.
pub(crate) fn expected_rev(app: &Path, config: &Config) -> anyhow::Result<Option<String>> {
    let Some(rust) = &config.rust else {
        return Ok(None);
    };
    let dirs: Vec<&str> = match rust {
        serde_json::Value::String(dir) => vec![dir.as_str()],
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .context("\"rust\" must be a string or an array of strings")
            })
            .collect::<anyhow::Result<_>>()?,
        _ => bail!("\"rust\" must be a string or an array of strings"),
    };
    let mut revisions: Vec<String> = Vec::new();
    for dir in dirs {
        let crate_dir = app.join(dir);
        let Some(lock_dir) = crate_dir
            .ancestors()
            .find(|dir| dir.join("Cargo.lock").is_file())
        else {
            bail!(
                "no Cargo.lock under {}; an unverified Rust crate must not run with another NSH revision",
                crate_dir.display()
            );
        };
        let lock = lock_dir.join("Cargo.lock");
        let text = fs::read_to_string(&lock).with_context(|| format!("read {}", lock.display()))?;
        let value: toml::Value =
            toml::from_str(&text).with_context(|| format!("{} is not TOML", lock.display()))?;
        let packages = value
            .get("package")
            .and_then(toml::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for package in packages {
            if package.get("name").and_then(toml::Value::as_str) != Some("nsh-core") {
                continue;
            }
            let Some(source) = package.get("source").and_then(toml::Value::as_str) else {
                continue;
            };
            if !source.starts_with("git+") {
                continue;
            }
            let Some((_, revision)) = source.rsplit_once('#') else {
                continue;
            };
            if revision.is_empty() {
                continue;
            }
            revisions.push(revision.to_owned());
        }
    }
    revisions.sort();
    revisions.dedup();
    match revisions.as_slice() {
        [] => Ok(None),
        [revision] => Ok(Some(revision.clone())),
        many => bail!(
            "the app's crates pin nsh-core at different revisions: {}",
            many.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: &str) -> Config {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_pin_is_one_exact_release() {
        let pinned = pin(&config(
            r#"{"toolchain":{"nsh":{"version":"0.6.1","repository":"acme/nsh"}}}"#,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(pinned.version, "0.6.1");
        assert_eq!(pinned.tag, "v0.6.1");
        assert_eq!(pinned.repository, "acme/nsh");
        let default = pin(&config(r#"{"toolchain":{"nsh":{"version":"v0.6.1"}}}"#))
            .unwrap()
            .unwrap();
        assert_eq!(default.tag, "v0.6.1");
        assert_eq!(
            default.repository,
            super::super::release::DEFAULT_REPOSITORY
        );
    }

    #[test]
    fn a_git_revision_pin_requires_a_full_commit() {
        let json = r#"{"toolchain":{"nsh":{"version":"rev=e0ca503108f5f6aa8501e17d342fc06de64f1975","repository":"YougLin-dev/nsh"}}}"#;
        let pinned = pin(&config(json)).unwrap().unwrap();
        assert_eq!(
            pinned.version,
            "rev=e0ca503108f5f6aa8501e17d342fc06de64f1975"
        );
        assert_eq!(pinned.repository, "YougLin-dev/nsh");
        assert!(
            pin(&config(
                r#"{"toolchain":{"nsh":{"version":"rev=e0ca503"}}}"#
            ))
            .is_err()
        );
    }

    #[test]
    fn ranges_and_channels_are_refused() {
        for version in [">=0.6", "0.6.*", "latest", "nightly", "0.6.1 latest"] {
            let json = format!(r#"{{"toolchain":{{"nsh":{{"version":{version:?}}}}}}}"#);
            assert!(pin(&config(&json)).is_err(), "{version}");
        }
        assert!(
            pin(&config(
                r#"{"toolchain":{"nsh":{"version":"0.6.1","repository":"../x"}}}"#
            ))
            .is_err()
        );
        assert!(
            pin(&config(
                r#"{"toolchain":{"nsh":{"version":"0.6.1","repository":"a/b/c"}}}"#
            ))
            .is_err()
        );
    }

    #[test]
    fn no_toolchain_means_no_pin() {
        assert!(pin(&config("{}")).unwrap().is_none());
        assert!(pin(&config(r#"{"toolchain":{}}"#)).unwrap().is_none());
    }

    #[test]
    fn the_rust_crates_lock_names_the_core_revision() {
        let app = std::env::temp_dir().join(format!(
            "nshw-manifest-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&app);
        fs::create_dir_all(app.join("native")).unwrap();
        fs::write(
            app.join("native/Cargo.lock"),
            r#"
version = 4
[[package]]
name = "nsh-core"
version = "0.6.0"
source = "git+https://github.com/longbridge/nsh?rev=abcdef#abcdef0123456789abcdef0123456789abcdef01"
[[package]]
name = "serde"
version = "1.0.0"
"#,
        )
        .unwrap();
        let config = config(r#"{"rust":"native"}"#);
        assert_eq!(
            expected_rev(&app, &config).unwrap().as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
        fs::write(
            app.join("native/Cargo.lock"),
            "version = 4\n[[package]]\nname = \"nsh-core\"\nversion = \"0.6.0\"\n",
        )
        .unwrap();
        assert_eq!(expected_rev(&app, &config).unwrap(), None);
        let _ = fs::remove_dir_all(&app);
    }

    #[test]
    fn a_rust_app_needs_a_readable_lockfile() {
        let app = std::env::temp_dir().join(format!("nshw-lock-required-{}", std::process::id()));
        let _ = fs::remove_dir_all(&app);
        fs::create_dir_all(app.join("native")).unwrap();
        let error = expected_rev(&app, &config(r#"{"rust":"native"}"#)).unwrap_err();
        assert!(error.to_string().contains("no Cargo.lock"), "{error}");
        let _ = fs::remove_dir_all(app);
    }

    #[test]
    fn an_explicit_app_wins_over_the_current_app() {
        let root = std::env::temp_dir().join(format!("nshw-find-app-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let current = root.join("current");
        let other = root.join("other");
        for app in [&current, &other] {
            fs::create_dir_all(app).unwrap();
            fs::write(app.join("nsh.json"), "{}").unwrap();
        }
        struct Restore(PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }
        let _restore = Restore(std::env::current_dir().unwrap());
        std::env::set_current_dir(&current).unwrap();
        let selected = find_app(&[OsString::from("run"), other.as_os_str().to_owned()]);
        assert_eq!(selected.unwrap(), other);
        let selected = find_app(&[
            OsString::from("run"),
            OsString::from("--name"),
            root.as_os_str().to_owned(),
            other.as_os_str().to_owned(),
        ]);
        assert_eq!(selected.unwrap(), other);
        let _restore = Restore(std::env::current_dir().unwrap());
        assert!(
            find_app(&[
                OsString::from("run"),
                OsString::from("--name"),
                root.as_os_str().to_owned(),
                OsString::from("/missing"),
            ])
            .unwrap_err()
            .to_string()
            .contains("not inside an app")
        );
        let original = std::env::current_dir().unwrap();
        fs::create_dir_all(root.join("unrelated")).unwrap();
        std::env::set_current_dir(root.join("unrelated")).unwrap();
        let _ = std::env::set_current_dir(original);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uppercase_commits_are_normalized_before_cache_selection() {
        let pin = pin(&config(
            r#"{"toolchain":{"nsh":{"version":"rev=C24505C146C8B7966D780C65879B5822D09BEAC5","repository":"YougLin-dev/nsh"}}}"#,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            pin.revision.as_deref(),
            Some("c24505c146c8b7966d780c65879b5822d09beac5")
        );
        assert_eq!(pin.version, "rev=c24505c146c8b7966d780c65879b5822d09beac5");
    }

    #[test]
    fn the_commands_src_default_discovers_its_app() {
        let root = std::env::temp_dir().join(format!("nshw-default-app-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/nsh.json"), "{}").unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();
        let selected = find_app(&[OsString::from("toolchain"), OsString::from("prefetch")]);
        std::env::set_current_dir(original).unwrap();
        assert_eq!(selected.unwrap(), root.join("src").canonicalize().unwrap());
        assert_eq!(
            app_argument(&[OsString::from("run")]),
            Some(OsString::from("."))
        );
        let _ = fs::remove_dir_all(root);
    }
}
