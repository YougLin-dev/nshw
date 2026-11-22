# Security

## Model

`nshw` selects and runs a toolchain; it is not a sandbox. The trust boundary is
the repository named by the project:

- `longbridge/nsh` is trusted implicitly.
- Another repository requires `NSH_TOOLCHAIN_ALLOW=owner/repo` or an explicit
  interactive confirmation.
- Release archives are checked against the published SHA-256 and must contain
  one `nsh/` directory with a regular `nsh/bin/nsh`.
- Git pins must resolve to the exact full commit and agree with the `nsh-core`
  revision in the application's `Cargo.lock`.
- `nshw` never silently substitutes a different release, commit or runtime.

Source builds run the selected checkout's own build system and therefore have
the same trust implications as running that repository's build locally.

## Reporting

Please report suspected vulnerabilities through GitHub's private security
advisory feature on this repository rather than a public issue. Include the
affected version, platform, exact pin, and a minimal reproduction when
possible.

Only the latest release is supported with security fixes.
