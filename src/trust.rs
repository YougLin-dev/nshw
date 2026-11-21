//! Policies for obtaining and executing a CLI from another repository.
//!
//! `longbridge/nsh` is the trust root. Any other repository is accepted only
//! after an explicit acknowledgment: `NSH_TOOLCHAIN_ALLOW=owner/repo,...` or
//! an interactive confirmation on a terminal. The check happens before a
//! download or a build, never after one.

use anyhow::{Result, bail};
use std::{
    env,
    io::{IsTerminal as _, Write as _},
};

pub(crate) fn check(pin: &crate::manifest::Pin) -> Result<()> {
    let repository = &pin.repository;
    if repository == crate::release::DEFAULT_REPOSITORY {
        return Ok(());
    }
    if env::var_os("NSH_TOOLCHAIN_ALLOW")
        .map(|list| {
            list.to_string_lossy()
                .split(',')
                .any(|r| r.trim() == repository)
        })
        .unwrap_or(false)
    {
        return Ok(());
    }
    if std::io::stdin().is_terminal() {
        eprintln!(
            "nshw: the project pins {repository}, not the default {}; executing a CLI from that repository requires confirmation",
            crate::release::DEFAULT_REPOSITORY
        );
        eprint!("proceed? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if answer.trim().eq_ignore_ascii_case("y") {
            return Ok(());
        }
    }
    bail!(
        "toolchain pin from {repository} requires trust: set `NSH_TOOLCHAIN_ALLOW={repository}` or confirm on a terminal"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Pin;

    fn pin(repository: &str) -> Pin {
        Pin {
            version: "0.6.1".into(),
            tag: "v0.6.1".into(),
            revision: None,
            repository: repository.into(),
        }
    }

    #[test]
    fn the_default_repository_is_implicitly_trusted() {
        assert!(check(&pin(crate::release::DEFAULT_REPOSITORY)).is_ok());
    }

    #[test]
    fn a_fork_is_rejected_without_an_ack_on_a_non_interactive_shell() {
        if std::env::var_os("NSH_TOOLCHAIN_ALLOW").is_none() && !std::io::stdin().is_terminal() {
            let error = check(&pin("acme/nsh")).unwrap_err();
            assert!(error.to_string().contains("requires trust"), "{error}");
        }
    }
}
