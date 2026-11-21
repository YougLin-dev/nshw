# nshw

Run exactly the Native Shell distribution your project declares. `nshw`
is independent of NSH: a version cannot fall back to another commit just
because it has the same package version.

```sh
nshw toolchain pin 0.6.1
nshw run src
```

## Declaration

`nsh.json` selects an exact release tag or a full 40-hex Git commit:

```json
{
  "rust": "src/native",
  "toolchain": {
    "nsh": {
      "version": "rev=e0ca503108f5f6aa8501e17d342fc06de64f1975",
      "repository": "YougLin-dev/nsh"
    }
  }
}
```

`repository` defaults to `longbridge/nsh`. Version ranges, channels, `latest`
and abbreviated commit hashes are rejected; an exact declaration is the only
input to selection.

- **Release:** download that tag's archive, verify the published SHA-256,
  validate its `nsh/` layout and atomically install it into the cache.
- **Git revision:** fetch that exact commit from the declared repository with
  the system `git`, then run its locked desktop builds: `nsh-cli` and this
  platform's development host and release runner. This is a complete desktop
  selection, not a CLI which silently borrows an older machine runtime.
  Optional iOS/Android/Web/GPUI/TUI components are not built or substituted
  from another release. The full commit owns its source, build output and
  serialization lock; inherited `CARGO_TARGET_DIR`/`CARGO_BUILD_TARGET`
  settings are removed so NSH's own target layout cannot be redirected.
- **Execution:** `exec` on Unix, synchronous execution with exit-status
  propagation on Windows. Original arguments are unchanged. No Cargo manifest
  or lockfile is modified. Only the explicit pin command edits `nsh.json`.

## Commands

All normal input, including `nshw help`, `nshw version` and
`nshw verify package.nshpkg`, belongs to the selected NSH CLI. The selector's
administration has its own unambiguous namespace:

| Command | Purpose |
| --- | --- |
| `nshw [nsh args...]` | Run the pinned CLI with unchanged arguments. |
| `nshw -- [nsh args...]` | Explicit NSH pass-through. |
| `nshw toolchain pin <release\|rev=SHA> [app]` | Update only the pin, preserving other settings. |
| `nshw toolchain prefetch [app]` | Obtain the complete selected distribution. |
| `nshw toolchain which [app]` | Print the selected CLI path. |
| `nshw toolchain status [app]` | Show declaration, cache, host and lock revision. |
| `nshw toolchain verify [app]` | Check the declaration against release metadata or the lock. |
| `nshw toolchain help` / `version` | Selector help or its version. |

Bare `nshw` describes these commands. When installed as an `nsh` alias, all
of its arguments go to NSH; use the `nshw` executable for administration.
An explicitly named app takes precedence over the current directory's app.

## Trust and offline use

Only `longbridge/nsh` is trusted implicitly. Any other repository requires an
`NSH_TOOLCHAIN_ALLOW=owner/repo` acknowledgement (comma-separated list), or
an interactive `y` confirmation. This applies before source acquisition,
download and execution. SHA-256 establishes archive integrity against the
declared release; it does not make a fork trustworthy.

The cache root is `NSH_CACHE`, else the platform cache directory followed by
`nsh`. Full repository/revision/host names are distinct. Cache hits execute
without network access. A Git pin requires `git`, `cargo` and a Rust
installation with the `stable` toolchain (invoked as `cargo +stable`). Release
downloads use the system `curl` for platform TLS. These are explicit host
prerequisites, not silent toolchain downloads.

Source builds require the same native SDKs and libraries as the selected NSH
revision's `xtask dist`. They can take considerable time and disk space.
Optional Android/Web distributions still depend on the selected checkout's
own requirements; no incompatible released component is substituted. With
neither the local cache nor network access available, a new pin cannot be
acquired.

Without a pin, the explicit `NSH_FALLBACK`, the standard installed
`~/.nsh`/`%LOCALAPPDATA%\nsh` distribution and then `nsh` on `PATH` are used.
Self-referential symlinks to `nshw` are rejected to prevent recursion.

## Installation

Choose a release of the selector itself (default below), or use Cargo:

```sh
curl -fsSL https://raw.githubusercontent.com/YougLin-dev/nshw/main/install.sh | sh
# or, from this source:
cargo install --path .
```

`install.sh` supports `NSHW_VERSION=...`, `NSHW_HOME=/prefix` and
`--set-default` on macOS/Linux. It verifies both downloads and the archive
layout before touching an existing install, stages the executable, then uses
atomic renames and symlink replacement. The `n` alias points to `nshw`.
`--set-default` additionally makes `nsh` a symlink, never overwriting a real
file. Without that option the installed `nsh` is unchanged. Add
`~/.nshw/bin` to `PATH` for your shell. Windows releases are installed from
source with `cargo install --path .` (or downloaded and extracted manually).

The repository and releases must be readable by the installation route used.
For private-repository distribution, deploy the installer and assets to an
authenticated mirror of your own; `NSHW_BASE_URL` selects that HTTP origin
for the script (the checksum requirements do not change).

## Verification

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
shellcheck install.sh
```

Tests include local HTTP release integrity checks, tar/zip behavior on the
current platform, the exact pin parser, trust, original-argv forwarding and
an actual Git/Cargo source fixture which builds a complete small distribution.
Platform tests run on macOS, Ubuntu and Windows; release artifacts are built
and smoke-tested on their native OS/architecture runners. Such fixtures do
not replace actually building and exercising a real NSH revision.
