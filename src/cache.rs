//! The toolchain cache: one directory per pinned version and machine,
//! downloaded once, verified, unpacked atomically and locked while it is
//! being written.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, bail};

use crate::{archive, exec, manifest, release, source};

/// `NSH_CACHE`, else the platform's cache directory, plus `nsh`.
pub(crate) fn cache_root() -> anyhow::Result<PathBuf> {
    if let Some(dir) = env::var_os("NSH_CACHE").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(windows) {
        env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        home.map(|home| home.join("Library/Caches"))
    } else {
        env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|home| home.join(".cache")))
    };
    Ok(base
        .context("no cache directory: set NSH_CACHE")?
        .join("nsh"))
}

/// The installed CLI of `pin` for `host`.
pub(crate) fn toolchain_dir(pin: &manifest::Pin, host: &str) -> anyhow::Result<PathBuf> {
    Ok(selected_dir(&cache_root()?, pin, host))
}

fn selected_dir(root: &Path, pin: &manifest::Pin, host: &str) -> PathBuf {
    root.join("toolchains")
        .join(pin.repository.replace('/', "__"))
        .join(&pin.version)
        .join(host)
}

/// Validate this binding before every execution, including complete caches.
fn source_revision(
    app: &Path,
    config: &manifest::Config,
    pin: &manifest::Pin,
) -> anyhow::Result<()> {
    if let Some(rev) = &pin.revision
        && let Some(expected) = manifest::expected_rev(app, config)?
        && expected != *rev
    {
        bail!("the toolchain pins {rev}, but Cargo.lock has nsh-core at {expected}");
    }
    Ok(())
}

fn normalize_revision(revision: &str) -> anyhow::Result<String> {
    let revision = revision.to_ascii_lowercase();
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Cargo.lock has an invalid full nsh-core revision");
    }
    Ok(revision)
}

/// The distributed desktop executables, never just a CLI.
fn release_files(dir: &Path, expected: Option<&str>) -> anyhow::Result<PathBuf> {
    let bin = dir.join("bin").join(exec::exe("nsh"));
    if !bin.is_file() {
        bail!("{} is missing the selected CLI", dir.display());
    }
    let os = std::env::consts::OS;
    for part in ["host", "app"] {
        let name = exec::exe(&format!("nsh-{os}-{part}"));
        let path = dir.join("runtime").join(os).join(name);
        if !path.is_file() {
            bail!(
                "{} is missing the desktop runtime {}",
                dir.display(),
                path.display()
            );
        }
    }
    let info: release::Info = match fs::read_to_string(dir.join(release::RELEASE_INFO)) {
        Ok(text) => serde_json::from_str(&text)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if expected.is_some() {
                bail!("the selected release has no source-commit metadata for its Rust app");
            }
            return Ok(bin);
        }
        Err(error) => return Err(error.into()),
    };
    let commit = info
        .commit
        .as_deref()
        .with_context(|| "the selected release has no source commit")?;
    let commit = commit.to_ascii_lowercase();
    if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{} has an invalid source commit", release::RELEASE_INFO);
    }
    if let Some(expected) = expected
        && !commit.eq_ignore_ascii_case(expected)
    {
        bail!(
            "the selected release is built from {commit}, but Cargo.lock has nsh-core at {expected}"
        );
    }
    Ok(bin)
}

/// Installs (once) and returns the `nsh` of `pin` for this machine.
pub(crate) fn ensure(
    app: &Path,
    config: &manifest::Config,
    pin: &manifest::Pin,
) -> anyhow::Result<PathBuf> {
    let host = release::host()?;
    source_revision(app, config, pin)?;
    if pin.revision.is_some() {
        return source::ensure_with_root(&cache_root()?, pin, host);
    }
    let expected = manifest::expected_rev(app, config)?;
    let expected = expected.as_deref().map(normalize_revision).transpose()?;
    let expected = expected.as_deref();
    crate::trust::check(pin)?;
    ensure_in(&cache_root()?, expected, pin, release::BASE)
}

fn ensure_in(
    root: &Path,
    expected: Option<&str>,
    pin: &manifest::Pin,
    base: &str,
) -> anyhow::Result<PathBuf> {
    let host = release::host()?;
    let dir = selected_dir(root, pin, host);
    if dir.join(".complete").is_file() {
        return release_files(&dir, expected);
    }

    let version_dir = dir.parent().context("toolchain directory")?;
    fs::create_dir_all(version_dir).with_context(|| format!("create {}", version_dir.display()))?;
    let lock_path = version_dir.join(format!(".{host}.lock"));
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("open {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("lock {}", lock_path.display()))?;
    if dir.join(".complete").is_file() {
        return release_files(&dir, expected);
    }

    let info = release::fetch_at(base, &pin.repository, &pin.tag)?;
    if let Some(expected) = expected {
        let info = info.as_ref().with_context(|| {
            format!(
                "{} has no {}; a Rust app requires release provenance",
                pin.tag,
                release::RELEASE_INFO
            )
        })?;
        if info.tag.as_deref() != Some(&pin.tag) {
            bail!(
                "{} does not identify the exact tag {}",
                release::RELEASE_INFO,
                pin.tag
            );
        }
        let commit = info
            .commit
            .as_deref()
            .with_context(|| format!("{} has no commit in {}", pin.tag, release::RELEASE_INFO))?;
        match commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            true if expected.eq_ignore_ascii_case(commit) => {}
            true => bail!(
                "{} is built from {commit}, but the app pins nsh-core at {expected}",
                pin.tag
            ),
            false => bail!("{} has an invalid source commit", release::RELEASE_INFO),
        }
    }

    let asset = release::asset(host);
    let part = version_dir.join(format!(".{host}.part"));
    let staging = version_dir.join(format!(".{host}.tmp"));
    let _ = fs::remove_dir_all(&staging);
    let result = (|| {
        release::download(base, &pin.repository, &pin.tag, &asset, &part)?;
        let unpacked = archive::unpack(&part, &staging)?;
        if let Some(info) = &info {
            fs::write(
                unpacked.join(release::RELEASE_INFO),
                serde_json::to_vec(info)?,
            )?;
        }
        release_files(&unpacked, expected)?;
        if dir.exists() {
            fs::remove_dir_all(&dir).with_context(|| format!("remove {}", dir.display()))?;
        }
        fs::rename(&unpacked, &dir).with_context(|| format!("install {}", dir.display()))?;
        fs::write(dir.join(".complete"), b"")
            .with_context(|| format!("write {}", dir.join(".complete").display()))?;
        anyhow::Ok(())
    })();
    let _ = fs::remove_file(&part);
    let _ = fs::remove_dir_all(&staging);
    result.with_context(|| {
        format!(
            "install nsh {} ({}) for {host}",
            pin.version, pin.repository
        )
    })?;
    release_files(&dir, expected)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::{
        collections::HashMap,
        io::{Read as _, Write as _},
        net::TcpListener,
        sync::Arc,
        thread,
    };

    #[cfg(unix)]
    use sha2::{Digest as _, Sha256};

    use super::*;

    #[test]
    fn a_release_from_an_untrusted_repository_is_rejected_before_fetching() {
        if std::env::var_os("NSH_TOOLCHAIN_ALLOW").is_some()
            || std::io::IsTerminal::is_terminal(&std::io::stdin())
        {
            return;
        }
        let pin = manifest::Pin {
            version: "0.6.1".into(),
            tag: "v0.6.1".into(),
            revision: None,
            repository: "acme/nsh".into(),
        };
        let root = std::env::temp_dir().join(format!(
            "nshw-cache-trust-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("app")).unwrap();
        fs::write(root.join("app/nsh.json"), "{}").unwrap();
        let config = manifest::read_config(&root.join("app")).unwrap();
        let error = ensure(&root.join("app"), &config, &pin).unwrap_err();
        assert!(error.to_string().contains("requires trust"), "{error}");
        assert!(
            !root.join("toolchains").exists(),
            "must not fetch an untrusted release"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_toolchain_lives_under_its_version_and_host() {
        let pin = manifest::Pin {
            version: "0.6.1".into(),
            tag: "v0.6.1".into(),
            revision: None,
            repository: release::DEFAULT_REPOSITORY.into(),
        };
        assert_eq!(
            selected_dir(Path::new("/cache"), &pin, "macos-arm64"),
            Path::new("/cache/toolchains/longbridge__nsh/0.6.1/macos-arm64")
        );
    }

    #[test]
    fn a_cached_release_checks_the_current_rust_revision_and_desktop_files() {
        let root = std::env::temp_dir().join(format!("nshw-release-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let distribution = root.join("toolchains/longbridge__nsh/0.6.1/macos-arm64");
        let os = std::env::consts::OS;
        fs::create_dir_all(distribution.join("bin")).unwrap();
        fs::write(distribution.join("bin").join(exec::exe("nsh")), b"cli").unwrap();
        assert!(release_files(&distribution, None).is_err());
        for part in ["host", "app"] {
            let name = exec::exe(&format!("nsh-{os}-{part}"));
            fs::create_dir_all(distribution.join("runtime").join(os)).unwrap();
            fs::write(distribution.join("runtime").join(os).join(name), b"runtime").unwrap();
        }
        fs::write(
            distribution.join(release::RELEASE_INFO),
            r#"{"commit":"ABCDEF0123456789ABCDEF0123456789ABCDEF01"}"#,
        )
        .unwrap();
        assert!(
            release_files(
                &distribution,
                Some("abcdef0123456789abcdef0123456789abcdef01")
            )
            .is_ok()
        );
        assert!(
            release_files(
                &distribution,
                Some("c24505c146c8b7966d780c65879b5822d09beac5")
            )
            .is_err()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_source_pin_and_lock_must_agree_before_even_a_cache_read() {
        let root = std::env::temp_dir().join(format!("nshw-source-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("native")).unwrap();
        fs::write(root.join("nsh.json"), r#"{"rust":"native"}"#).unwrap();
        fs::write(root.join("native/Cargo.lock"), format!("version=4\n[[package]]\nname=\"nsh-core\"\nsource=\"git+https://github.com/YougLin-dev/nsh#{}\"\n", "e0ca503108f5f6aa8501e17d342fc06de641975")).unwrap();
        let config = manifest::read_config(&root).unwrap();
        let pin = manifest::pin(&manifest::read_config(&root).unwrap()).unwrap();
        assert!(pin.is_none());
        let pin = manifest::pin(&serde_json::from_value(serde_json::json!({"toolchain":{"nsh":{"version":"rev=C24505C146C8B7966D780C65879B5822D09BEAC5","repository":"YougLin-dev/nsh"}}})).unwrap()).unwrap().unwrap();
        assert!(source_revision(&root, &config, &pin).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_downloads_verifies_and_unpacks_a_release() {
        let dir = std::env::temp_dir().join(format!(
            "nshw-cache-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("app/native")).unwrap();
        let commit = "abcdef0123456789abcdef0123456789abcdef01";
        fs::write(
            dir.join("app/nsh.json"),
            r#"{"toolchain":{"nsh":{"version":"0.6.1"}},"rust":"native"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("app/native/Cargo.lock"),
            format!(
                "version = 4\n[[package]]\nname = \"nsh-core\"\nversion = \"0.6.0\"\nsource = \"git+https://github.com/longbridge/nsh?rev={commit}#{commit}\"\n"
            ),
        )
        .unwrap();

        let host = release::host().unwrap();
        let asset = release::asset(host);
        let archive = tarball();
        let hash = format!("{:x}", Sha256::digest(&archive));
        let base_path = "/longbridge/nsh/releases/download/v0.6.1";
        let files: HashMap<String, Vec<u8>> = [
            (format!("{base_path}/{asset}"), archive),
            (
                format!("{base_path}/{asset}.sha256"),
                format!("{hash}  {asset}\n").into_bytes(),
            ),
            (
                format!("{base_path}/{}", release::RELEASE_INFO),
                format!(r#"{{"version":"0.6.0","commit":"{commit}","tag":"v0.6.1"}}"#).into_bytes(),
            ),
        ]
        .into_iter()
        .collect();
        let (base, _server) = serve(files);

        let app = dir.join("app");
        let config = manifest::read_config(&app).unwrap();
        let pin = manifest::pin(&config).unwrap().unwrap();
        let root = dir.join("cache");
        let bin = ensure_in(&root, Some(commit), &pin, &base).unwrap();
        assert_eq!(
            bin,
            root.join("toolchains/longbridge__nsh/0.6.1")
                .join(host)
                .join("bin/nsh")
        );
        assert_eq!(fs::read(&bin).unwrap(), b"fake");
        assert!(bin.is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn tarball() -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        let os = std::env::consts::OS;
        let files = [
            "nsh/bin/nsh".to_owned(),
            format!("nsh/runtime/{os}/nsh-{os}-host"),
            format!("nsh/runtime/{os}/nsh-{os}-app"),
        ];
        for path in files {
            let data = b"fake";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, path, &data[..]).unwrap();
        }
        let tar = tar.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar).unwrap();
        encoder.finish().unwrap()
    }

    /// A local release host: `GET <path>` answers from `files`.
    #[cfg(unix)]
    fn serve(files: HashMap<String, Vec<u8>>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let files = Arc::new(files);
        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let files = Arc::clone(&files);
                let mut buffer = [0u8; 4096];
                let Ok(read) = stream.read(&mut buffer) else {
                    continue;
                };
                let request = String::from_utf8_lossy(&buffer[..read]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let response = match files.get(&path) {
                    Some(body) => {
                        let mut response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(body);
                        response
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = stream.write_all(&response);
            }
        });
        (format!("http://{addr}"), handle)
    }
}
