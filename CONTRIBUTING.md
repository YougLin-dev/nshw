# Contributing

## Development

Requirements:

- Rust 1.89 or newer
- `shellcheck` for the installer
- Git and Cargo for the source-build integration test

Run the same checks as CI:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
shellcheck install.sh
```

`cargo test` includes unit tests, the public CLI tests in `tests/cli.rs`, and a
real Git/Cargo source fixture. The source fixture downloads and builds a small
fake distribution and can take longer than the other tests.

## Design constraints

- Selection is exact: a release tag or a full 40-hex commit. Do not add
  ranges, channels, `latest`, or abbreviated commits.
- A mismatch between `nsh.json` and `Cargo.lock`'s `nsh-core` revision is an
  error, never a fallback.
- Release assets must pass SHA-256 and archive-layout checks before entering
  the cache.
- Non-default repositories require an explicit trust acknowledgement.
- `nshw` never rewrites an application manifest or lockfile. Only
  `nshw toolchain pin` edits `nsh.json`, and it preserves unrelated settings.
- Keep the selector independent of NSH's source tree and releases.

## Release

1. Update `version` in `Cargo.toml` and the default in `install.sh`.
2. Commit the version change and push it.
3. Run the Release workflow with the matching `v<version>` and full commit.
4. Verify the tag, target commit, 12 assets, checksums and a public
   `cargo install --git ... --tag v<version> --locked`.
