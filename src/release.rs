//! Release acquisition: exact GitHub asset URLs, curl's platform HTTPS,
//! and a SHA-256 check before any bytes are accepted into the cache.

use anyhow::{Context as _, bail};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub(crate) const DEFAULT_REPOSITORY: &str = "longbridge/nsh";
pub(crate) const RELEASE_INFO: &str = "nsh-release.json";
pub(crate) const BASE: &str = "https://github.com";
const USER_AGENT: &str = concat!("nshw/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Deserialize)]
pub(crate) struct Info {
    #[serde(default)]
    pub(crate) version: Option<String>,
    #[serde(default)]
    pub(crate) commit: Option<String>,
    #[serde(default)]
    pub(crate) tag: Option<String>,
}

pub(crate) fn host() -> anyhow::Result<&'static str> {
    host_for(std::env::consts::OS, std::env::consts::ARCH)
}

fn host_for(os: &str, arch: &str) -> anyhow::Result<&'static str> {
    Ok(match (os, arch) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x86_64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-arm64",
        ("windows", "x86_64") => "windows-x86_64",
        ("windows", "aarch64") => "windows-arm64",
        (os, arch) => bail!("no prebuilt nsh for {os} {arch}"),
    })
}

pub(crate) fn asset(host: &str) -> String {
    if cfg!(windows) {
        format!("nsh-{host}.zip")
    } else {
        format!("nsh-{host}.tar.gz")
    }
}

pub(crate) fn url(base: &str, repository: &str, tag: &str, name: &str) -> String {
    format!(
        "{}/{repository}/releases/download/{tag}/{name}",
        base.trim_end_matches('/')
    )
}

pub(crate) fn fetch(repository: &str, tag: &str) -> anyhow::Result<Option<Info>> {
    fetch_at(BASE, repository, tag)
}

pub(crate) fn fetch_at(base: &str, repository: &str, tag: &str) -> anyhow::Result<Option<Info>> {
    let url = url(base, repository, tag, RELEASE_INFO);
    let response = get(&url)?;
    match response.status {
        404 => return Ok(None),
        status if !(200..300).contains(&status) => bail!("GET {url}: HTTP {status}"),
        _ => {}
    }
    let body = String::from_utf8(fs::read(response.body_path())?)?;
    Ok(Some(
        serde_json::from_str(&body).context("read the release description")?,
    ))
}

pub(crate) fn download(
    base: &str,
    repository: &str,
    tag: &str,
    name: &str,
    to: &Path,
) -> anyhow::Result<()> {
    let url = url(base, repository, tag, name);
    let sum_url = format!("{url}.sha256");
    let sum_response = get(&sum_url)?;
    if !(200..300).contains(&sum_response.status) {
        bail!("GET {sum_url}: HTTP {}", sum_response.status);
    }
    let sum = String::from_utf8(fs::read(sum_response.body_path())?)?;
    let expected = sum
        .split_whitespace()
        .next()
        .filter(|hash| hash.len() == 64)
        .context("checksum sidecar")?
        .to_ascii_lowercase();
    let response = get(&url)?;
    if !(200..300).contains(&response.status) {
        bail!("GET {url}: HTTP {}", response.status);
    }
    let response = Response::from_curl(response)?;
    let mut reader = fs::File::open(response.body_path())?;
    let mut file = fs::File::create(to).with_context(|| format!("create {}", to.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 256 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])?;
    }
    file.sync_all()?;
    if format!("{:x}", hasher.finalize()) != expected {
        bail!("the download of {url} does not match its SHA-256");
    }
    Ok(())
}

/// Curl supplies the platform's TLS. Bytes live in a private, unique
/// temporary directory, not a predictable file in shared /tmp.
struct Response {
    body_path: PathBuf,
    _body: tempfile::TempPath,
    status: u16,
}

impl Response {
    fn body_path(&self) -> &Path {
        &self.body_path
    }

    fn from_curl(response: Self) -> anyhow::Result<Self> {
        fs::metadata(response.body_path()).context("curl downloaded no body")?;
        Ok(response)
    }
}

fn get(url: &str) -> anyhow::Result<Response> {
    let file = tempfile::NamedTempFile::new()?;
    let directory = file
        .path()
        .parent()
        .context("private temporary directory")?;
    fs::create_dir_all(directory)?;
    let body_path = file.path().to_owned();
    let body = file.into_temp_path();
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=http,https",
            "--proto-redir",
            "=http,https",
            "--max-redirs",
            "10",
            "--connect-timeout",
            "30",
            "--header",
        ])
        .arg(format!("User-Agent: {USER_AGENT}"))
        .args(["--output"])
        .arg(&body_path)
        .args(["--write-out", "%{http_code}"])
        .arg(url)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("GET {url}; install curl"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let response = Response {
        body_path,
        _body: body,
        status: text.trim().parse().unwrap_or(0),
    };
    if !output.status.success() || !(200..600).contains(&response.status) {
        bail!("GET {url}: curl failed with {}", output.status);
    }
    Response::from_curl(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_release_urls_match_nsh_archives() {
        assert_eq!(host_for("macos", "aarch64").unwrap(), "macos-arm64");
        assert_eq!(host_for("windows", "x86_64").unwrap(), "windows-x86_64");
        assert!(host_for("freebsd", "x86_64").is_err());
        assert_eq!(
            url(BASE, DEFAULT_REPOSITORY, "v0.6.1", "nsh-macos-arm64.tar.gz"),
            "https://github.com/longbridge/nsh/releases/download/v0.6.1/nsh-macos-arm64.tar.gz"
        );
        let expected = if cfg!(windows) {
            "nsh-macos-arm64.zip"
        } else {
            "nsh-macos-arm64.tar.gz"
        };
        assert_eq!(asset("macos-arm64"), expected);
    }
}
