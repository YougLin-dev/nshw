//! Exact commits produce core desktop distributions: CLI, host and runner.
//! Each repository/SHA owns its checkout and target. The source lock covers
//! through acquisition, build and installation. The system Git supplies
//! HTTPS and credential support; a Cargo checkout from another crate never
//! substitutes for the repository the project selected.

use crate::{exec, manifest, trust};
use anyhow::{Context as _, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub(crate) fn dir(root: &Path, pin: &manifest::Pin, host: &str) -> PathBuf {
    root.join("toolchains")
        .join(pin.repository.replace('/', "__"))
        .join(&pin.version)
        .join(host)
}

fn staging(parent: &Path, label: &str) -> Result<PathBuf> {
    let path = parent.join(format!(".{label}.{}.tmp", std::process::id()));
    if path.exists() {
        fs::remove_dir_all(&path)?;
    }
    fs::create_dir_all(&path)?;
    Ok(path)
}

pub(crate) fn ensure_with_root(root: &Path, pin: &manifest::Pin, host: &str) -> Result<PathBuf> {
    ensure_at(
        root,
        pin,
        host,
        &format!("https://github.com/{}.git", pin.repository),
    )
}

fn ensure_at(root: &Path, pin: &manifest::Pin, host: &str, url: &str) -> Result<PathBuf> {
    trust::check(pin)?;
    let rev = pin
        .version
        .strip_prefix("rev=")
        .context("a source pin requires rev=SHA")?;
    let workspace = root
        .join("sources")
        .join(pin.repository.replace('/', "__"))
        .join(rev);
    fs::create_dir_all(&workspace)?;
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(workspace.join(".lock"))?;
    lock.lock().context("lock the pinned source")?;
    let distribution = dir(root, pin, host);
    let bin = distribution.join("bin").join(exec::exe("nsh"));
    fs::create_dir_all(distribution.parent().context("toolchain parent")?)?;
    if complete(&distribution, rev) {
        return Ok(bin);
    }
    let source = checked_out(&workspace, url, rev)?;
    let staging = staging(distribution.parent().context("toolchain parent")?, host)?;
    let result = install(&source, &staging)
        .and_then(|()| Ok(fs::write(staging.join(".revision"), rev.as_bytes())?));
    if result.is_ok() {
        if distribution.exists() {
            fs::remove_dir_all(&distribution)?;
        }
        fs::rename(&staging, &distribution)?;
    }
    let _ = fs::remove_dir_all(staging);
    result?;
    Ok(bin)
}

fn complete(distribution: &Path, rev: &str) -> bool {
    let os = std::env::consts::OS;
    distribution.join("bin").join(exec::exe("nsh")).is_file()
        && fs::read_to_string(distribution.join(".revision")).is_ok_and(|r| r == rev)
        && ["host", "app"].iter().all(|part| {
            distribution
                .join("runtime")
                .join(os)
                .join(exec::exe(&format!("nsh-{os}-{part}")))
                .is_file()
        })
}

fn git(directory: &Path, args: &[&str]) -> Result<()> {
    run(Command::new("git").current_dir(directory).args(args))
}

/// Fetch the full SHA directly, even when no branch or tag advertises it.
fn checked_out(workspace: &Path, url: &str, rev: &str) -> Result<PathBuf> {
    let source = workspace.join("source");
    if source.exists()
        && fs::read_to_string(workspace.join("origin")).is_ok_and(|u| u.trim() == url)
        && git(&source, &["cat-file", "-e", &format!("{rev}^{{commit}}")]).is_ok()
    {
        let head = Command::new("git")
            .current_dir(&source)
            .args(["rev-parse", "HEAD"])
            .output()?;
        if head.status.success() && String::from_utf8_lossy(&head.stdout).trim() == rev {
            return Ok(source);
        }
        fs::remove_dir_all(&source)?;
    }
    let staging = staging(workspace, "source")?;
    git(&staging, &["init", "--quiet"])?;
    git(&staging, &["remote", "add", "origin", url])?;
    git(&staging, &["fetch", "--depth=1", "origin", rev])?;
    let commit = Command::new("git")
        .current_dir(&staging)
        .args(["rev-parse", "FETCH_HEAD^{commit}"])
        .output()?;
    if !commit.status.success() || String::from_utf8_lossy(&commit.stdout).trim() != rev {
        bail!("the repository did not provide the exact commit {rev}");
    }
    git(&staging, &["checkout", "--detach", "--force", rev])?;
    if source.exists() {
        fs::remove_dir_all(&source)?;
    }
    fs::rename(staging, &source)?;
    fs::write(workspace.join("origin"), format!("{url}\n"))?;
    Ok(source)
}

/// Build NSH's core desktop distribution from this exact source revision.
/// Optional iOS/Android/Web/GPUI/TUI runtimes are not silently substituted
/// from an unrelated release; the desktop toolchain doesn't require them.
fn install(source: &Path, staging: &Path) -> Result<()> {
    let os = std::env::consts::OS;
    run(Command::new("cargo")
        .arg("+stable")
        .current_dir(source)
        .args(["build", "--locked", "--release", "-p", "nsh-cli", "-p"])
        .arg(format!("nsh-{os}-host"))
        .arg("-p")
        .arg(format!("nsh-{os}-app"))
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit()))?;
    let target = source.join("target/release");
    fs::create_dir_all(staging.join("bin"))?;
    copy_executable(
        &target.join(exec::exe("nsh")),
        &staging.join("bin").join(exec::exe("nsh")),
    )?;
    let runtime = staging.join("runtime").join(os);
    fs::create_dir_all(&runtime)?;
    for part in ["host", "app"] {
        let name = exec::exe(&format!("nsh-{os}-{part}"));
        copy_executable(&target.join(&name), &runtime.join(&name))?;
    }
    Ok(())
}

fn copy_executable(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to).with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(to)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(to, permissions)?;
    }
    Ok(())
}

fn run(command: &mut Command) -> Result<()> {
    let status = command.status().context("run the source tool")?;
    if !status.success() {
        bail!("source tool failed with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_install_is_a_distribution_with_an_exact_revision() -> Result<()> {
        let root = std::env::temp_dir().join(format!("nshw-source-layout-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let distribution = root.join("dist/nsh");
        let os = std::env::consts::OS;
        fs::create_dir_all(distribution.join("bin"))?;
        fs::create_dir_all(distribution.join("runtime").join(os))?;
        fs::write(distribution.join("bin").join(exec::exe("nsh")), b"cli")?;
        for part in ["host", "app"] {
            fs::write(
                distribution
                    .join("runtime")
                    .join(os)
                    .join(exec::exe(&format!("nsh-{os}-{part}"))),
                b"host",
            )?;
        }
        assert!(!complete(&distribution, "abc"));
        fs::write(distribution.join(".revision"), "abc")?;
        assert!(complete(&distribution, "abc"));
        assert!(!complete(&distribution, "def"));
        fs::remove_file(
            distribution
                .join("runtime")
                .join(os)
                .join(exec::exe(&format!("nsh-{os}-host"))),
        )?;
        assert!(!complete(&distribution, "abc"));
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    /// Real Git and Cargo, with a repository whose shape matches NSH's own
    /// xtask distribution contract. This is not a local-path libgit2 test.
    #[test]
    fn a_remote_sha_builds_and_transparently_executes() -> Result<()> {
        let base = std::env::temp_dir().join(format!("nshw-source-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let upstream = base.join("upstream");
        super::super::test_support::source_fixture(&upstream)?;
        let output = Command::new("git")
            .current_dir(&upstream)
            .args(["rev-parse", "HEAD"])
            .output()?;
        let rev = String::from_utf8(output.stdout)?.trim().to_owned();
        let pin = manifest::Pin {
            version: format!("rev={rev}"),
            tag: rev.clone(),
            repository: crate::release::DEFAULT_REPOSITORY.into(),
        };
        let cache = base.join("cache");
        let host = crate::release::host()?;
        let bin = ensure_at(&cache, &pin, host, upstream.to_str().unwrap())?;
        assert!(complete(bin.parent().unwrap().parent().unwrap(), &rev));
        let output = Command::new(&bin)
            .args(["verify", "package.nshpkg"])
            .output()?;
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout)?.trim(),
            "verify package.nshpkg"
        );
        assert_eq!(
            ensure_at(&cache, &pin, host, upstream.to_str().unwrap())?,
            bin
        );
        let _ = fs::remove_dir_all(base);
        Ok(())
    }
}
