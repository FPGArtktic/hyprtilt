---
description: Development stages of hyprtilt and what is done.
---

<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Roadmap

hyprtilt is built in stages. Each stage ends with a report of what works,
how to test it and which decisions were made.

| Stage | Scope | State |
|---|---|---|
| 0. Bootstrap | Cargo workspace, pinned build image, `justfile`, CI, contribution rules | :material-check-circle:{ style="color: #00c781" } done |
| 1. Research and design | [Verified notes on Hyprland's Lua API](../hyprland-lua-api.md), [architecture decision record](../adr/0001-architecture.md) | :material-check-circle:{ style="color: #00c781" } done |
| 2. Core and `list` | Monitor model, geometry, Lua and hyprlang managed blocks, IPC, `list`, `doctor` | :material-check-circle:{ style="color: #00c781" } done |
| 3. TUI | Canvas, selection, moving, rotation, details panel, saving the block | :material-check-circle:{ style="color: #00c781" } done |
| 4. Apply and the rest of the TUI | Apply with countdown and rollback, modes, refresh rates, scale, VRR, snapping, profiles, adoption | :material-check-circle:{ style="color: #00c781" } done |
| 5. CLI | All subcommands, `--json`, `--dry-run`, exit codes, man page, completions | :material-check-circle:{ style="color: #00c781" } done |
| 6. Packaging and documentation | AUR, `.deb`, `.rpm`, release workflow, this site | :material-check-circle:{ style="color: #00c781" } done |
| 7. Release | `v0.1.0`, checks on real hardware, demo GIF | :material-check-circle:{ style="color: #00c781" } done (crates.io on the next run) |

## Non-goals for 0.x

- A hotplug or lid daemon.
- Workspace planning.
- Integrations with panels or shells.
- Compositors other than Hyprland.
