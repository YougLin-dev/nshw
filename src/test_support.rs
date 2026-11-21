//! Test-only source fixture. It implements NSH's distribution contract,
//! while keeping the build tiny enough to run in the platform test matrix.

use std::{fs, path::Path, process::Command};

pub(crate) fn source_fixture(root: &Path) -> anyhow::Result<()> {
    for directory in ["cli", "host", "runner"] {
        fs::create_dir_all(root.join("packages").join(directory).join("src"))?;
    }
    fs::create_dir_all(root.join(".cargo"))?;
    fs::write(
        root.join(".cargo/config.toml"),
        "[alias]\nxtask = \"run -q -p xtask --\"\n",
    )?;
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers=[\"packages/*\"]\nresolver=\"3\"\n",
    )?;
    fs::write(root.join(".gitignore"), "/dist\n/target\n")?;
    let os = std::env::consts::OS;
    let packages = [
        ("cli", "nsh-cli".to_owned(), "nsh".to_owned()),
        ("host", format!("nsh-{os}-host"), format!("nsh-{os}-host")),
        ("runner", format!("nsh-{os}-app"), format!("nsh-{os}-app")),
    ];
    for (directory, package, binary) in packages {
        let directory = root.join("packages").join(directory);
        fs::write(
            directory.join("Cargo.toml"),
            format!(
                "[package]\nname=\"{package}\"\nversion=\"0.1.0\"\nedition=\"2024\"\n\n[[bin]]\nname=\"{binary}\"\npath=\"src/main.rs\"\n"
            ),
        )?;
        fs::write(
            directory.join("src/main.rs"),
            "fn main() { println!(\"{}\", std::env::args().skip(1).collect::<Vec<_>>().join(\" \")); }\n",
        )?;
    }
    git(root, &["init", "--quiet"])?;
    git(root, &["add", "."])?;
    git(
        root,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.com",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    )?;
    assert!(
        Command::new("cargo")
            .arg("+stable")
            .current_dir(root)
            .args(["generate-lockfile", "--quiet"])
            .status()?
            .success()
    );
    git(root, &["add", "Cargo.lock"])?;
    git(
        root,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.com",
            "commit",
            "--quiet",
            "-m",
            "lock",
        ],
    )?;
    // HEAD no longer advertises this committed source revision.
    git(root, &["checkout", "--quiet", "--orphan", "fixture-other"])?;
    git(root, &["commit", "--allow-empty", "--quiet", "-m", "other"])?;
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> anyhow::Result<()> {
    anyhow::ensure!(
        Command::new("git")
            .current_dir(root)
            .args(args)
            .status()?
            .success(),
        "test Git command failed"
    );
    Ok(())
}
