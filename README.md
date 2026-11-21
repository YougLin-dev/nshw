# nshw

Run the Native Shell CLI your project pins. Every time, with the original
arguments, without a second `nsh` executable name to learn.

```sh
nshw pin 0.6.1
nshw run src
# Outputs nsh 0.6.1 even if the system nsh is older.

nshw pin rev=e0ca503108f5f6aa8501e17d342fc06de64f1975
# Set the repository in nsh.json and allow it explicitly (below) for a fork.
```

## The contract

- `toolchain.nsh.version` is an exact release, or `rev=<full-40-hex-Git-SHA>`.
- `toolchain.nsh.repository` defaults to `longbridge/nsh`.
- Release selections download the GitHub prebuilt archive and verify its
  `.sha256`. Git-revision selections use an existing Cargo checkout when
  available, otherwise clone/fetch the requested repository, then run
  `cargo build --release --locked -p nsh-cli`.
- The selected CLI is run using `exec` on Unix (process replacement), and
  synchronously with exit-status propagation on Windows. The original argv is
  passed through untouched.
- Apps and their `Cargo.toml` / `Cargo.lock` are never modified.

## Commands

| Command | Purpose |
| --- | --- |
| `nshw [nsh args...]` | Run the pinned CLI with those args. |
| `nshw pin <release\|rev=SHA> [app]` | Write the pin, preserving all other `nsh.json` keys. |
| `nshw prefetch [app]` | Install/build the selected CLI if absent. |
| `nshw which [app]` | Print the selected CLI path. |
| `nshw status [app]` | Show the pin, host, cache and the locked nsh-core revision when readable. |
| `nshw verify [app]` | Check the release metadata against Cargo.lock. |
| `nshw help` / `nshw version` | Tool help / tool version. |

Example `nsh.json`:

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

## Trust and offline use

The implicit trust root is `longbridge/nsh`. A non-default repository requires
`NSH_TOOLCHAIN_ALLOW=owner/repo` (comma-separated when needed), or an explicit
`y` confirmation on a terminal. The trust check is done before any download or
build. Checksums verify archive integrity against the stated release, not
whether that repository is trustworthy.

Cache root: `NSH_CACHE`, else the platform cache directory followed by `nsh`.
Separate repositories, revisions and host triples never share an installation.
Release downloads are checksum-verified; source builds are locked selections.
After the first successful install, ready toolchains run without network access.

For a Git pin on a new machine, `git`, `cargo` and a Rust toolchain are needed
for the first build. Cargo's existing checkout is copied rather than changed.
A source pin does not promise a prebuilt artifact and cannot work offline
unless the source or a Cargo checkout is already available.

## Installation

Install the release you choose into `~/.nshw/bin`, or build from source:

```sh
curl -fsSL https://raw.githubusercontent.com/YougLin-dev/nshw/main/install.sh | sh
# or
cargo install --path .
```

`install.sh` takes `NSHW_VERSION=...`, `NSHW_HOME=/prefix`, and
`--set-default` (adds a `nsh` symlink ahead of the existing distribution).
Installing `nshw` alone never replaces the system `nsh`.

## Verification

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The test suite includes local HTTP release serving with checksums, tar/zip
unpacking, exact-pin validation, source building from a local Cargo checkout,
installation and an execution check. GitHub migrations or NSH source-build
requirements are independent of this selector; it does not simplify NSH's
platform prerequisites. Linux and Windows differ in archive format and how
process replacement is implemented.
