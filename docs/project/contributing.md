---
description: How to build, test and contribute to hyprtilt.
---

<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Contributing

The full guide is
[CONTRIBUTING.md](https://github.com/FPGArtktic/hyprtilt/blob/main/CONTRIBUTING.md)
in the repository. In short:

- Everything builds and is checked with `just`, on the host or inside the
  pinned build image (`just podman ci`), so local results match CI.
- `just ci` runs formatting, clippy with warnings as errors, the tests, the
  API documentation, the dependency audit and the coverage thresholds
  (`hyprtilt-core` at least 85 %, the workspace at least 70 %).
- Commits follow the Linux kernel process: `area: imperative description`,
  a body that explains the problem and the change, and a `Signed-off-by`
  line under the
  [Developer Certificate of Origin](https://github.com/FPGArtktic/hyprtilt/blob/main/DCO).
- Behaviour of Hyprland is never guessed: it comes from
  [the Hyprland API notes](../hyprland-lua-api.md), which cite Hyprland's
  sources, and new facts are added there first.
- Tests never touch a real Hyprland configuration: they use fixtures and an
  in-memory Hyprland.

## Architecture in brief

| Part | Where | What |
|---|---|---|
| Core library | `crates/hyprtilt-core` | monitor model, geometry, Lua and hyprlang managed blocks, IPC, the apply state machine, profiles, settings, `doctor` |
| Binary | `crates/hyprtilt` | command line, terminal interface |

The core never touches the terminal; the interface's state and keys are
plain data, the controller carries out effects with a clock passed in, so
both run in tests without a terminal or a compositor. The reasons for every
decision are in [ADR 0001](../adr/0001-architecture.md).

## Trying the interface without Hyprland

The hidden option `--fake-hyprland SETUP.json` replaces the compositor with
an in-memory one. `demo/` holds a setup with three monitors:

```sh
cp -r demo /tmp/hyprtilt-demo
cargo run -- --fake-hyprland /tmp/hyprtilt-demo/fake.json
```
