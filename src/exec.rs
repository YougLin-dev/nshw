//! Running the selected CLI, and the installed distribution for a project
//! without a pin.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(windows)]
use anyhow::Context as _;
use anyhow::bail;

/// The executable's file name on this platform (`nsh.exe` on Windows).
pub(crate) fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn pinned_command(bin: &Path, args: &[OsString]) -> Command {
    let mut command = Command::new(bin);
    command.args(args).env_remove("NSH_RUNTIME");
    command
}

/// A pinned distribution owns its adjacent runtime; an inherited override
/// must not replace the selected desktop host or release runner.
pub(crate) fn exec_pinned(bin: &Path, args: &[OsString]) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let error = pinned_command(bin, args).exec();
        Err(anyhow::Error::new(error).context(format!("exec {}", bin.display())))
    }
    #[cfg(windows)]
    {
        let status = pinned_command(bin, args)
            .status()
            .with_context(|| format!("run {}", bin.display()))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

/// Replaces this process with `bin` on Unix, or runs it and exits with its
/// status on Windows.
pub(crate) fn exec(bin: &Path, args: &[OsString]) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let error = std::process::Command::new(bin).args(args).exec();
        Err(anyhow::Error::new(error).context(format!("exec {}", bin.display())))
    }
    #[cfg(windows)]
    {
        let status = std::process::Command::new(bin)
            .args(args)
            .status()
            .with_context(|| format!("run {}", bin.display()))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

/// Runs the installed distribution when the project does not pin nsh.
pub(crate) fn fallback(args: &[OsString]) -> anyhow::Result<()> {
    let Some(bin) = fallback_bin() else {
        bail!(
            "no nsh.json with a toolchain.nsh pin here, and no installed nsh distribution; \
             try nshw help"
        );
    };
    exec(&bin, args)
}

/// The distribution `NSH_FALLBACK` names, else the installer's `~/.nsh`
/// (`%LOCALAPPDATA%\nsh`), unless that is nshw itself.
fn candidate(path: PathBuf, current: Option<&PathBuf>) -> Option<PathBuf> {
    let path = fs::canonicalize(path).ok()?;
    (path.is_file() && Some(&path) != current).then_some(path)
}

/// Prefer an explicitly selected distribution, then PATH (Homebrew, Scoop,
/// mise and similar). Exclude this wrapper's canonical executable to avoid
/// recursively launching it through its own `nsh` symlink.
fn fallback_bin() -> Option<PathBuf> {
    let current = std::env::current_exe()
        .ok()
        .and_then(|path| fs::canonicalize(path).ok());
    if let Some(explicit) = std::env::var_os("NSH_FALLBACK")
        .map(PathBuf::from)
        .and_then(|path| candidate(path, current.as_ref()))
    {
        return Some(explicit);
    }
    let installed = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("nsh/bin/nsh.exe"))
    } else {
        std::env::var_os("HOME").map(|dir| PathBuf::from(dir).join(".nsh/bin/nsh"))
    };
    if let Some(installed) = installed.and_then(|path| candidate(path, current.as_ref())) {
        return Some(installed);
    }

    std::env::split_paths(&std::env::var_os("PATH")?)
        .find_map(|dir| candidate(dir.join(exe("nsh")), current.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pinned_command_clears_a_foreign_runtime_but_keeps_other_argv() {
        let mut command = pinned_command(
            Path::new("/pinned/bin/nsh"),
            &[
                OsString::from("test"),
                OsString::from("--junit"),
                OsString::from("report.xml"),
            ],
        );
        let command = command.env("NSH_TEST_ARGUMENT", "unchanged");
        assert_eq!(
            command.get_program(),
            Path::new("/pinned/bin/nsh").as_os_str()
        );
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["test", "--junit", "report.xml"].map(OsString::from)
        );
        assert!(
            command
                .get_envs()
                .any(|(name, value)| name == "NSH_RUNTIME" && value.is_none())
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "NSH_TEST_ARGUMENT")
                .and_then(|(_, value)| value),
            Some(std::ffi::OsStr::new("unchanged"))
        );
    }
}
