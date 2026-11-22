# Changelog

All notable changes to this project are documented here.

## [Unreleased]

## [0.2.0] - 2026-11-22

### Added

- Fall back to the installed NSH distribution when an invocation is outside
  any app, instead of failing on a missing `nsh.json`.
- `nshw toolchain --help`/`-h` and `--version`/`-V`.
- Download and source-build progress on stderr.
- Archive link and traversal rejection, with regression tests.
- Public CLI integration tests and an MSRV CI job.
- Contributor, security and changelog documentation.

### Changed

- Split the selector administration commands into `src/command.rs`.
- Installer `--help`/`--version`, leading-`v` normalization, and a direct
  executable mode.
- Release workflows now gate on tests and require the tag to match
  `Cargo.toml`; actions and the shellcheck action are pinned.

## [0.1.1] - 2026-11-21

### Changed

- Published the release from the same commit as its tag.
- Updated the default installer release to 0.1.1.

## [0.1.0] - 2026-11-21

### Added

- Exact release-tag and full-commit selection from `nsh.json`.
- SHA-256-verified release downloads and locked source builds.
- Per-revision/per-host cache with locks and atomic installation.
- `toolchain pin`, `status`, `verify`, `prefetch` and `which`.
- Trust acknowledgement for non-default repositories.
- macOS, Linux and Windows release builds with architecture-specific assets.
