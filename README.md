<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# hyprtilt

[![CI](https://github.com/FPGArtktic/hyprtilt/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/FPGArtktic/hyprtilt/actions/workflows/ci.yml)
[![Documentation](https://img.shields.io/readthedocs/hyprtilt)](https://hyprtilt.readthedocs.io)
[![MSRV](https://img.shields.io/badge/rust-1.88%2B-blue?logo=rust)](Cargo.toml)
[![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-blue)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-linux%20amd64%20%7C%20arm64-lightgrey)](#installation)
![Made in Poland](https://img.shields.io/badge/made%20in-Poland-DC143C?labelColor=white)

A keyboard-first terminal interface and command line for arranging
[Hyprland](https://hypr.land) monitors, which edits **your own**
configuration file in place, inside a managed block — both the Lua
configuration (`hl.monitor({...})`) and hyprlang (`monitor=...`). One static
Rust binary, no daemon, no sidecar files.

The Lua backend needs Hyprland 0.55 or newer, which is where the Lua
configuration arrived; the hyprlang backend covers the older versions that
Ubuntu and Debian still ship.

> [!IMPORTANT]
> **hyprtilt is under construction and has no release yet.** The binary
> currently prints its version and nothing else. What is finished is the
> research, the design and the whole build, packaging and release pipeline.
> Follow the [roadmap](https://hyprtilt.readthedocs.io/en/latest/project/roadmap/).

## Contents

- [Why hyprtilt](#why-hyprtilt)
- [How it will work](#how-it-will-work)
- [How it compares](#how-it-compares)
- [Installation](#installation)
- [Building from source](#building-from-source)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [Security](#security)
- [License](#license)
- [Author](#author)

## Why hyprtilt

Hyprland 0.55 introduced a Lua configuration, and modern setups define
monitors like this:

```lua
hl.monitor({ output = "HDMI-A-1", mode = "2560x1440@144", position = "0x0", scale = 1, transform = 1 })
```

Every existing tool leaves that file alone and writes a file of its own,
which you then have to `source` or `require` from your configuration. That
breaks down when a dotfile framework such as
[Caelestia](https://github.com/caelestia-dots/caelestia) owns
`~/.config/hypr/` and overwrites it on update, and it means the rules you
wrote by hand and the rules the tool wrote live in different places.

hyprtilt is **config-file-first**: your configuration file is the source of
truth.

- It edits the file you already use, inside `-- BEGIN hyprtilt (managed)` …
  `-- END hyprtilt`. Everything outside the block stays **byte for byte**
  identical, and saving an unchanged layout writes nothing at all.
- It finds `hl.monitor`, `monitor=` and `monitorv2` rules you already have
  and offers to adopt them into the block. `unmanage` gives them back.
- It knows where your configuration really lives, including Caelestia's
  `hypr-user.lua`, and warns when Hyprland would load a different file.
- Changes are verified against the live state over Hyprland's IPC, and are
  rolled back unless you confirm them.

## How it will work

```lua
-- Your own settings stay exactly as they are.
hl.config({ input = { repeat_delay = 500, repeat_rate = 30 } })

-- BEGIN hyprtilt (managed)
hl.monitor({ output = "HDMI-A-1", mode = "2560x1440@144", position = "0x0", scale = 1, transform = 1 })
hl.monitor({ output = "eDP-1", mode = "1920x1080@144", position = "1440x1335", scale = 1 })
hl.monitor({ output = "DP-1", mode = "2560x1440@179.95", position = "3360x975", scale = 1, vrr = 2 })
-- END hyprtilt

return {} -- the block always goes before a top-level return
```

```sh
hyprtilt                            # the terminal interface
hyprtilt list --json                # live state and what the file says
hyprtilt rotate HDMI-A-1 90         # for a keybinding
hyprtilt apply --confirm-timeout 15 # apply, verify, roll back unless confirmed
hyprtilt doctor                     # is the right file being loaded?
```

## How it compares

Checked on 2026-09-28 against the sources of each project, not their
marketing. hyprtilt's column is its design; see the status note above for
what is already built.

| | hyprtilt | hyprmoncfg | hyprmon | nwg-displays | kanshi |
|---|---|---|---|---|---|
| Edits your own config in place | **Yes** | No | hyprlang only | No | No |
| Managed block with markers | **Yes** | No | Include line only | No | — |
| Preserves the rest of the file byte for byte | **Yes** | — | No | — | — |
| Adopts your existing rules | **Yes** | No | No | No | No |
| Caelestia (`hypr-user.lua`) | **Yes** | No | No | No | No |
| Lua configuration | Yes | Yes | Yes | Yes | — |
| hyprlang configuration | Yes | Yes | Yes | Yes | — |
| Verifies the live state after applying | Yes | Yes | No | No | — |
| Automatic rollback | Yes | Yes | Manual undo | No | — |
| Keyboard-first (vim keys) | Yes | Arrows and mouse | Yes | GUI | — |
| Per-output CLI (`rotate`, `move`, …) | **Yes** | No | No | No | No |
| Profiles | Yes | Yes | Yes | No | Yes |
| Hotplug daemon | **No** | Yes | No | No | Yes |
| Workspace planning | **No** | Yes | No | No | No |
| Single static binary | Yes | Two plus a daemon | Yes | Python and GTK | Yes |

A hotplug daemon and workspace planning are deliberate non-goals of the 0.x
releases: hyprmoncfg does them well, and hyprtilt is not trying to replace
it on that axis.

## Installation

> Nothing is released yet, so the packages below install a binary that only
> prints its version.

**Arch Linux (AUR).** The recipe in
[`packaging/aur/hyprtilt-git/`](packaging/aur/hyprtilt-git) builds the
`main` branch. It is not published to the AUR yet; until it is, build it
from this repository:

```sh
git clone https://github.com/FPGArtktic/hyprtilt
cd hyprtilt/packaging/aur/hyprtilt-git && makepkg -si
```

**Debian, Ubuntu and derivatives.** Once releases start, a `.deb` will be
attached to every release; the binary is static, so it installs on older
LTS releases too (tested on Ubuntu 22.04, Ubuntu 24.04 and Debian 12).

```sh
sudo apt install ./hyprtilt_<version>_amd64.deb
```

**Fedora and other RPM systems.** An `.rpm` will be attached as well (tested
on Fedora 43).

```sh
sudo dnf install ./hyprtilt-<version>-1.x86_64.rpm
```

**Cargo.**

```sh
cargo install hyprtilt        # once it is published
```

After installing, `hyprtilt --version` prints the version.

## Building from source

You need [`just`](https://github.com/casey/just) and Podman; everything else
comes from a pinned Arch Linux build image, so your machine and CI use the
same tools.

```sh
just image          # once, and after containers/build/ changes
just podman ci      # fmt, clippy, tests, docs, dependency checks, coverage
just podman release-build   # static musl binaries for amd64 and arm64
```

With a local Rust toolchain you can also run `just ci` directly.
[CONTRIBUTING.md](CONTRIBUTING.md) lists every target.

## Documentation

<https://hyprtilt.readthedocs.io>, built from [`docs/`](docs). Of special
interest:

- [ADR 0001: architecture](docs/adr/0001-architecture.md) — every design
  decision and why.
- [Hyprland monitor configuration API](docs/hyprland-lua-api.md) — about 350
  facts about Hyprland 0.56.2, each with a link to the source it comes from.
  This is the project's source of truth; the code never guesses.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Commits follow the Linux kernel
process applied to Rust, and every commit is signed off under the
[Developer Certificate of Origin](DCO).

## Security

Report vulnerabilities privately, as described in [SECURITY.md](SECURITY.md).
Do not open a public issue.

## License

hyprtilt is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License, version 3 only
(`GPL-3.0-only`), as published by the Free Software Foundation. See
[LICENSE](LICENSE) for the full text. The same license covers the
documentation.

## Author

Mateusz Okulanis <FPGArtktic@outlook.com>
