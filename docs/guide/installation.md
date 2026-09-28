---
description: Install hyprtilt from the AUR, a .deb or .rpm package, with Cargo, or from source.
---

<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Installation

hyprtilt is one static binary without runtime dependencies. It runs on
x86_64 and aarch64 Linux.

!!! tip "Latest release"

    The packages below are attached to
    [every release](https://github.com/FPGArtktic/hyprtilt/releases/latest);
    `v0.1.0` is the current one, and both AUR packages are published.

## Arch Linux (AUR)

Two recipes live in [`packaging/aur/`](https://github.com/FPGArtktic/hyprtilt/tree/main/packaging/aur):

| Package | Builds |
|---|---|
| [`hyprtilt`](https://aur.archlinux.org/packages/hyprtilt) | the latest release, from the source archive of its tag |
| [`hyprtilt-git`](https://aur.archlinux.org/packages/hyprtilt-git) | the `main` branch |

```sh
paru -S hyprtilt        # or: yay -S hyprtilt
```

To build a recipe straight from the repository instead:

```sh
git clone https://github.com/FPGArtktic/hyprtilt
cd hyprtilt/packaging/aur/hyprtilt-git
makepkg -si
```

Both install the man page and completions for bash, zsh and fish. The
optional `lua` package lets hyprtilt check the syntax of a Lua file before
writing it.

## Debian, Ubuntu and derivatives

Every release attaches `.deb` packages for amd64 and arm64. The binary is
static, so the package installs on older releases as well (tested on
Ubuntu 22.04, Ubuntu 24.04 and Debian 12):

```sh
curl -LO https://github.com/FPGArtktic/hyprtilt/releases/download/v0.1.0/hyprtilt_0.1.0-1_amd64.deb
sudo apt install ./hyprtilt_0.1.0-1_amd64.deb
```

## Fedora and other RPM systems

```sh
curl -LO https://github.com/FPGArtktic/hyprtilt/releases/download/v0.1.0/hyprtilt-0.1.0-1.x86_64.rpm
sudo dnf install ./hyprtilt-0.1.0-1.x86_64.rpm
```

## Release archives

`hyprtilt-<version>-x86_64-unknown-linux-musl.tar.gz` (and the aarch64
one) contain the binary, the man page, the completions, the license and
the README. `SHA256SUMS` lists the checksums of every artifact and is
signed with [cosign](https://github.com/sigstore/cosign) without a key:

```sh
sha256sum --check --ignore-missing SHA256SUMS
cosign verify-blob --bundle SHA256SUMS.cosign.bundle \
  --certificate-identity-regexp '^https://github.com/FPGArtktic/hyprtilt/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
```

## Cargo

Once published to crates.io:

```sh
cargo install --locked hyprtilt
```

## From source

With Rust 1.88 or newer:

```sh
git clone https://github.com/FPGArtktic/hyprtilt
cd hyprtilt
cargo build --release --locked -p hyprtilt
install -Dm755 target/release/hyprtilt ~/.local/bin/hyprtilt
```

The man page and the completions come from the binary itself:

```sh
hyprtilt man > hyprtilt.1
hyprtilt completions bash > ~/.local/share/bash-completion/completions/hyprtilt
hyprtilt completions zsh  > ~/.zfunc/_hyprtilt
hyprtilt completions fish > ~/.config/fish/completions/hyprtilt.fish
```

## Requirements

- Hyprland 0.55 or newer for the Lua configuration; the hyprlang
  configuration works with older versions as well.
- A terminal of at least 80×24 cells for the interface (smaller terminals
  work with less detail).
- `luac` from Lua 5.5 or 5.4, optional: used to check Lua files before
  they are written.
