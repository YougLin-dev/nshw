//! Source-built toolchains for full Git-commit pins.
//!
//! The source is obtained before any build from a Cargo checkout when
//! available, then from the pinned GitHub repository. The output is locked,
//! staged and atomically renamed into the platform cache.

use crate::{exec, manifest, trust};
use anyhow::{Context as _, Result, bail};
use git2::Repository;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn dir(root: &Path, pin: &manifest::Pin, host: &str) -> PathBuf {
    root.join("toolchains")
        .join(pin.repository.replace('/', "__"))
        .join(&pin.version)
        .join(host)
}

pub(crate) fn binary(root: &Path, pin: &manifest::Pin, host: &str) -> PathBuf {
    dir(root, pin, host).join("bin").join(exec::exe("nsh"))
}

pub(crate) fn ensure_with_root(root: &Path, pin: &manifest::Pin, host: &str) -> Result<PathBuf> {
    trust::check(pin)?;
    let rev = pin
        .version
        .strip_prefix("rev=")
        .context("source::ensure_with_root requires a rev pin")?;
    let dir = dir(root, pin, host);
    let bin = binary(root, pin, host);
    if dir.join(".complete").is_file() && bin.is_file() {
        return Ok(bin);
    }
    let version_dir = dir.parent().context("toolchain parent")?;
    fs::create_dir_all(version_dir)?;
    let lock_path = version_dir.join(format!(".{host}.lock"));
    let lock_file = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock_file.lock()?;
    if dir.join(".complete").is_file() && bin.is_file() {
        return Ok(bin);
    }

    let sources = root.join("sources").join(pin.repository.replace('/', "__"));
    fs::create_dir_all(&sources)?;
    let url = format!("https://github.com/{}.git", pin.repository);
    let source = if let Some(existing) = cached_checkout(rev) {
        cached_checkout_copy(&existing, &sources, rev)?
    } else {
        let clone = sources.join("nsh");
        if clone.exists() {
            let repo = Repository::open(&clone)?;
            let mut remote = repo.find_remote("origin")?;
            remote.fetch(&[rev], None, None)?;
            clone
        } else {
            Repository::clone(&url, &clone)?;
            clone
        }
    };
    let repo = Repository::open(&source)?;
    let object = repo.revparse_single(rev)?;
    let commit = object.peel_to_commit()?;
    if commit.id().to_string() != rev {
        bail!("revision {rev} is not the fetched full commit");
    }
    repo.checkout_tree(&object, Some(git2::build::CheckoutBuilder::new().force()))?;
    repo.set_head_detached(commit.id())?;

    let status = Command::new("cargo")
        .args(["build", "--release", "--locked", "-p", "nsh-cli"])
        .current_dir(&source)
        .status()
        .context("run cargo build")?;
    if !status.success() {
        bail!("cargo build --release --locked -p nsh-cli failed");
    }
    let built = source.join("target/release").join(exec::exe("nsh"));
    if !built.is_file() {
        bail!("built NSH not found at {}", built.display());
    }

    let staging = version_dir.join(format!(".{host}.tmp"));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(staging.join("bin"))?;
    fs::copy(&built, staging.join("bin").join(exec::exe("nsh")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perms = fs::metadata(staging.join("bin").join(exec::exe("nsh")))?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(staging.join("bin").join(exec::exe("nsh")), perms)?;
    }
    fs::write(staging.join(".complete"), b"")?;
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::rename(&staging, &dir)?;
    Ok(bin)
}

fn cached_checkout(rev: &str) -> Option<PathBuf> {
    let cargo = env_path("CARGO_HOME").or_else(|| env_path("HOME").map(|h| h.join(".cargo")))?;
    for outer in fs::read_dir(cargo.join("git/checkouts")).ok()?.flatten() {
        for entry in fs::read_dir(outer.path()).ok()?.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let repo = if let Ok(repo) = Repository::open(&path) {
                repo
            } else {
                continue;
            };
            let Some(head) = repo.head().ok().and_then(|h| h.target()) else {
                continue;
            };
            if head.to_string() == rev {
                return Some(path);
            }
        }
    }
    None
}

fn cached_checkout_copy(from: &Path, base: &Path, rev: &str) -> Result<PathBuf> {
    let to = base.join(format!("nsh-{}", &rev[..7]));
    if !to.exists() {
        git2::build::RepoBuilder::new().clone(&from.to_string_lossy(), &to)?;
    }
    Ok(to)
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use git2::Signature;

    use super::*;

    fn environment() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn an_exact_revision_is_built_and_cached_from_a_cargo_checkout() {
        let _guard = environment().lock().unwrap();
        let base = std::env::temp_dir().join(format!(
            "nshw-source-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&base);
        let repo_dir = base.join("cargo/git/checkouts/nsh-fixture/abcdef0");
        fs::create_dir_all(repo_dir.join("src")).unwrap();
        fs::write(
            repo_dir.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"nsh-cli\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("Cargo.toml"),
            "[package]\nname = \"nsh-cli\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[[bin]]\nname = \"nsh\"\npath = \"src/main.rs\"\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("src/main.rs"),
            "fn main() { println!(\"fixture-nsh\"); }\n",
        )
        .unwrap();
        let repo = Repository::init(&repo_dir).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("Cargo.toml")).unwrap();
        index.add_path(Path::new("Cargo.lock")).unwrap();
        index.add_path(Path::new("src/main.rs")).unwrap();
        let id = index.write_tree().unwrap();
        let signature = Signature::now("nshw test", "test@example.com").unwrap();
        let tree = repo.find_tree(id).unwrap();
        let commit = repo
            .commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
            .unwrap()
            .to_string();

        let cache = base.join("cache");
        // These process variables are scoped to this serial fixture test.
        unsafe {
            std::env::set_var("NSH_CACHE", &cache);
            std::env::set_var("CARGO_HOME", base.join("cargo"));
        }
        let pin = manifest::Pin {
            version: format!("rev={commit}"),
            tag: format!("rev={commit}"),
            repository: super::super::release::DEFAULT_REPOSITORY.into(),
        };
        let host = super::super::release::host().unwrap();
        let first = ensure_with_root(&cache, &pin, host).unwrap();
        assert!(first.is_file());
        let second = ensure_with_root(&cache, &pin, host).unwrap();
        assert_eq!(first, second);
        let output = Command::new(first).output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "fixture-nsh"
        );
        unsafe {
            std::env::remove_var("NSH_CACHE");
            std::env::remove_var("CARGO_HOME");
        }
        let _ = fs::remove_dir_all(&base);
    }
}
