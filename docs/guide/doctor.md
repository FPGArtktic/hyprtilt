---
description: What hyprtilt doctor checks and how to fix each problem.
---

<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Troubleshooting

```console
$ hyprtilt doctor
[ ok ] Hyprland 0.56.2 is running with a lua configuration and 3 monitor(s)
[ ok ] the target's language (lua) is the one Hyprland runs
[info] Hyprland's main configuration: /home/you/.config/hypr/hyprland.lua (found in the configuration directories)
[ ok ] hyprtilt edits /home/you/.config/caelestia/hypr-user.lua (lua; the Caelestia preset: the file require("hypr-user") loads)
[ ok ] managed block on lines 5-9 with 3 rule(s)
[ ok ] Hyprland watches /home/you/.config/caelestia/hypr-user.lua, so it loads it
[ ok ] Hyprland reports no configuration errors
[ ok ] the live state matches the block for 3 output(s)
[ ok ] Lua 5.5.0 checks new content before it is written (/usr/bin/luac)

No problems found.
```

`[warn]` and `[FAIL]` lines are problems; `doctor` then exits with code 7.
`doctor --json` gives each check a stable `id`, listed below.

| Id | What it checks | What to do |
|---|---|---|
| `hyprland` | Hyprland answers on its socket | start Hyprland, or run hyprtilt inside its session (`HYPRLAND_INSTANCE_SIGNATURE` must be set) |
| `backend` | the file's language is the one Hyprland runs (Lua needs 0.55 or newer) | choose the right file with `--file` or `target` in `config.toml` |
| `hyprlang-deprecated` | Hyprland 0.56.1 and newer announce the removal of hyprlang | move to the Lua configuration when convenient |
| `main-config` | where Hyprland's main configuration is | information |
| `target` | which file hyprtilt edits and why; warns when another `hypr-user.lua` shadows it | remove the shadowing file or point `--file` at the one that is loaded |
| `target-file` | the file exists, can be read, and its managed block is valid | fix the reported line; rules inside the block must be literals |
| `writable` | the file can be written (not read-only, not in `/nix/store`) | change the permissions, or change the Nix configuration instead |
| `outside-rules` | monitor rules outside the block | `hyprtilt adopt`, or leave them |
| `conflicting-rule` | a rule outside the block for the same output that merges with (Lua) or competes with (hyprlang) the block's rule | adopt it or remove it |
| `shadowing-rule` | a rule outside the block, with another selector, that also applies to a monitor of the block and wins (a later rule; in hyprlang any `monitorv2` block) | adopt it or remove it; changes to the block cannot take effect |
| `loaded` | Hyprland watches the file, so it loads it | make your configuration load the file (`require`, `source`, `dofile`) |
| `config-errors` | Hyprland reports no configuration errors | `hyprctl configerrors` shows them |
| `live-state` | what Hyprland shows matches the block | `hyprtilt apply` applies the file; `hyprtilt list` shows the differences |
| `layout` | overlaps (fail), gaps the pointer cannot cross, scales Hyprland adjusts (warn), modes the monitor does not list (info) | fix the layout in the interface |
| `selectors` | `desc:` selectors that match more than one monitor (a description matches by its beginning) | write the full description, serial number included |
| `syntax-check` | a Lua compiler for the syntax check before writing | install `lua` (5.4 or 5.5) |

## Common situations

**"Hyprland cannot be reached."** hyprtilt talks to Hyprland through
`$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`. Over SSH
or from a service these variables are missing; export them from the
Hyprland session. Without Hyprland, the file can still be edited with
`--dry-run` or in the interface.

**"Hyprland does not watch the file."** Hyprland reloads the files it read.
If the file hyprtilt edits is not among them, your configuration does not
load it, and changes to it cannot take effect.

**A change is rolled back although it looks right.** Look at the reported
difference. A refresh rate the monitor does not list becomes a custom mode,
which Hyprland may run at another rate; the monitor panel of the interface
lists the rates the monitor offers. A scale that does not divide the resolution evenly is adjusted by
Hyprland; the interface offers only scales that are not.
