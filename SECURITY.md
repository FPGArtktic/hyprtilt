<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Security policy

## Supported versions

Security fixes are made on the `main` branch and published in a new
release. Older releases do not get fixes; upgrade to the latest one.

| Version | Supported |
|---|---|
| Latest release | Yes |
| `main` branch, including the AUR package `hyprtilt-git` | Yes |
| Older releases | No |

`hyprtilt --version` prints the version of a binary.

## Reporting a vulnerability

**Do not report vulnerabilities in public issues, pull requests or
discussions.** Report them privately, in one of these ways:

- **GitHub:** open the
  [Security tab](https://github.com/FPGArtktic/hyprtilt/security) of the
  repository and click **Report a vulnerability**, or go straight to the
  [private report form](https://github.com/FPGArtktic/hyprtilt/security/advisories/new).
- **E-mail:** if GitHub is not an option, send an e-mail to Mateusz
  Okulanis, [FPGArtktic@outlook.com](mailto:FPGArtktic@outlook.com), with
  "hyprtilt security" in the subject.

## What to include

- The version (`hyprtilt --version`), the installation method, the
  Hyprland version (`hyprctl version`) and the distribution.
- The kind of problem and its impact: what an attacker controls, and what
  they can achieve.
- Steps to reproduce, ideally with a minimal configuration file.
- Whether the problem is already public, and any disclosure deadline you
  have in mind.

## What to expect

hyprtilt is maintained by one person in their spare time, so these are
goals rather than guarantees:

- **Acknowledgement** within 7 days.
- **Assessment** within 14 days: accepted or not, and how severe.
- **Fix** in a release, normally within 90 days of the report.
- **Disclosure** as a GitHub security advisory once the fix is released,
  crediting you unless you prefer otherwise.

## Scope

hyprtilt edits configuration files and talks to the Hyprland IPC sockets.
Reports are especially welcome about:

- writes outside the configured target file and its backups, or through
  symbolic links to unexpected places;
- configuration content, output names or monitor descriptions that inject
  code into the generated Lua or hyprlang, or control sequences into the
  terminal;
- loss of content outside the managed block;
- the release process: checksums, cosign signatures and packages.

Out of scope: vulnerabilities in Hyprland itself, which go to the Hyprland
project, and in Rust crates, which go to their maintainers (please still
tell us if hyprtilt is affected).
