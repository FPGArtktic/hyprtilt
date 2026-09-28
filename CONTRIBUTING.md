<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Contributing to hyprtilt

Thank you for helping. This guide covers building, testing, code style and
commit messages. The same commands run on developer machines and in CI, inside
the same pinned build image, so a change that passes locally passes in CI.

By contributing you agree that your work is licensed under the project
license, `GPL-3.0-only` (see [LICENSE](LICENSE)), and you certify the
[Developer Certificate of Origin](DCO) by signing off every commit.

## Contents

- [Prerequisites](#prerequisites)
- [Building and testing](#building-and-testing)
- [Build image](#build-image)
- [Workspace layout](#workspace-layout)
- [Coding style](#coding-style)
- [File headers](#file-headers)
- [Commit messages](#commit-messages)
- [Testing rules](#testing-rules)
- [Test coverage](#test-coverage)
- [The Hyprland API](#the-hyprland-api)
- [Pull requests](#pull-requests)

## Prerequisites

- Linux, Git, Bash and [`just`](https://github.com/casey/just).
- Podman (rootless works) for the container targets. The build image is
  x86_64 only; aarch64 binaries are cross-compiled.
- Optional, for quick local loops: Rust with `rustup`. `rust-toolchain.toml`
  pins the toolchain; `rust-version` in `Cargo.toml` is the MSRV.

All other tools (cargo-llvm-cov, cargo-deny, cargo-deb, cargo-generate-rpm,
namcap, git-cliff, vhs, mkdocs-material) come at pinned versions from the
build image.

## Building and testing

Every target runs on the host (`just <target>`) or in the build image
(`just podman <target>...`):

| Target | What it does |
|---|---|
| `just build` | Build every crate and target (debug) |
| `just test` | Unit, integration and documentation tests |
| `just lint` | `cargo fmt --check` and `cargo clippy -D warnings` |
| `just doc` | API documentation with warnings as errors |
| `just deny` | Dependency licenses, advisories, bans, sources |
| `just cov` | Line coverage with the thresholds below |
| `just msrv` | Build check with the minimum supported Rust version |
| `just commits [BASE]` | Check commit messages of `BASE..HEAD` |
| `just ci` | Everything the main CI job runs |
| `just release-build` | Static musl binaries for x86_64 and aarch64 in `dist/` |
| `just image` | Build the build image |

Before sending a change, run:

```sh
just image          # once, and after containers/build/ changes
just podman ci commits
```

## Build image

`containers/build/Containerfile` defines the image. It is based on
`archlinux:base-devel`, pinned by tag and digest, and installs every package
from the Arch Linux Archive snapshot of the same day, so a rebuild gives the
same tools. The Rust toolchain comes from `rust-toolchain.toml` through
rustup, plus the MSRV toolchain.

`just podman` runs the image rootless with `--userns=keep-id`: you are the
user `builder` (UID 1000) inside, who may run `pacman` through `sudo` for
`makepkg`. Cargo's registry, the target directory and pacman's package cache
live on the named volumes `hyprtilt-cargo`, `hyprtilt-target` and
`hyprtilt-pacman`.

CI uses the same image from `ghcr.io/fpgartktic/hyprtilt-build`, pinned by
digest. The workflow `build-image.yml` rebuilds and publishes it when the
Containerfile or `rust-toolchain.toml` changes; the new digest is then pinned
in the workflows in a separate commit.

To update the pins, change `ARCH_IMAGE_TAG`, `ARCH_IMAGE_DIGEST` and
`ARCH_ARCHIVE_DATE` together (the archive date is the day of the image tag).

## Workspace layout

```
crates/hyprtilt-core   model, geometry, Lua and hyprlang parser/generator with
                       the managed block, profiles, IPC, apply/rollback logic
crates/hyprtilt        the binary: TUI (ratatui) and CLI (clap), a thin layer
                       over hyprtilt-core
docs/                  documentation site (MkDocs), ADRs in docs/adr/
containers/build/      the build image
packaging/             AUR, Debian and RPM packaging
scripts/               helper scripts used by the justfile and CI
```

`hyprtilt-core` never talks to a terminal and never reads the wall clock
directly: time and IPC are injected, so every behaviour is testable.

## Coding style

The rules follow the Linux kernel's `Documentation/rust/coding-guidelines.rst`
and `Documentation/process/coding-style.rst`, applied to this project:

- Default `rustfmt` formatting. `cargo clippy --all-targets -- -D warnings`
  with the `pedantic` group enabled; the few allowed lints are listed with a
  reason in the workspace `Cargo.toml`.
- `unsafe` code is forbidden. `hyprtilt-core` denies missing documentation.
- Every public item has a `///` comment. Add `# Examples`, `# Errors` and
  `# Panics` sections wherever they apply.
- Comments explain *why*, not *what*.
- Short functions and shallow nesting. Prefer early returns.
- Errors carry enough context to act on them (which file, which line, which
  output). The binary never panics on bad input; it prints an error and exits
  with a documented exit code.

## File headers

Every source file starts with an SPDX license identifier and a copyright
line, in the comment syntax of the file:

```rust
// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
```

## Commit messages

The rules follow the kernel's `Documentation/process/submitting-patches.rst`.
`just commits` checks them, and CI runs it on every push and pull request.

### Format

```
area: imperative description without a trailing period

Problem: what is wrong or missing, and why it matters.

Rationale and what changed, at the level of intent. Do not restate the
diff; the diff is right below.

Signed-off-by: Your Name <you@example.org>
```

- One logical change per commit. Every commit builds and passes the tests,
  so `git bisect` works.
- The subject is at most 72 characters, the body wraps at 75 columns
  (trailers and URLs may be longer).
- Sign off every commit (`git commit -s`). The sign-off must match the
  commit author.
- Rebase, never merge. Squash review fixups into the commits they fix.

### Areas

| Prefix | Scope |
|---|---|
| `core` | model and shared types in `hyprtilt-core` |
| `geometry` | transforms, scale, logical sizes, layout arithmetic |
| `lua` | Lua parser, generator and managed block |
| `hyprlang` | hyprlang parser, generator and managed block |
| `ipc` | Hyprland sockets and the IPC abstraction |
| `apply` | live apply, verification and rollback |
| `tui` | terminal user interface |
| `cli` | command line interface |
| `docs` | documentation, README, ADRs |
| `ci` | GitHub Actions and CI scripts |
| `pkg` | AUR, Debian and RPM packaging |
| `build` | Cargo workspace, build image, justfile |

### Fixes tag

A commit that fixes a regression names the commit that introduced it:

```
Fixes: 0123456789ab ("lua: preserve bytes outside managed block")
```

## Testing rules

- Tests never touch the real Hyprland session or the user's configuration.
  They use fixture files and the mock implementation of `HyprlandIpc`.
- The maintainer's layout is a mandatory fixture: `HDMI-A-1` rotated
  (`transform = 1`) at `0x0`, `eDP-1` at `1440x1335`, `DP-1` at `3360x975`.
  Tests assert the data (for example that `eDP-1` and `DP-1` share the bottom
  edge `y = 2415`), not a description.
- Round trips: parse, generate, parse again and compare. Everything outside
  the managed block must be byte-for-byte identical after a write.
- Timing: the apply/rollback state machine takes time as an input, so tests
  drive the countdown without sleeping.
- TUI: render tests with `ratatui::backend::TestBackend` at 80×24 and larger.

## Test coverage

`just cov` measures line coverage with `cargo-llvm-cov` and enforces:

| Scope | Minimum |
|---|---|
| `hyprtilt-core` | 85 % |
| Whole workspace | 70 % |

CI fails below either threshold and uploads the report to Codecov.

## The Hyprland API

Do not guess the Hyprland API. [`docs/hyprland-lua-api.md`](docs/hyprland-lua-api.md)
records every fact the code relies on, with links to the Hyprland sources at a
fixed version. If something is missing, research it in the sources and extend
that document in the same pull request.

## Pull requests

- Base pull requests on `main`, keep them focused, and rebase on `main`
  before review.
- CI must be green: lint, tests on x86_64 and aarch64, MSRV, docs, coverage,
  dependency checks and commit messages.
