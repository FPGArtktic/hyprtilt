---
description: Which file hyprtilt edits, what the managed block is, Lua, hyprlang and Caelestia specifics, and config.toml.
---

<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Configuration files

hyprtilt edits one file: your own Hyprland configuration, or the file it
loads your settings from. It never creates a file of its own for Hyprland
to include.

## Which file

The first of these wins:

1. `--file PATH` on the command line.
2. `target` in [`config.toml`](#settings).
3. **Caelestia**: when Hyprland's main configuration is Lua, `require`s
   `hypr-user` and adds a `caelestia` directory to `package.path`,
   hyprtilt edits the `hypr-user.lua` that `require` finds (normally
   `~/.config/caelestia/hypr-user.lua`). Caelestia owns `~/.config/hypr/`
   and overwrites it on update, but never touches this file.
4. Hyprland's main configuration: the path given with `--config` to the
   running Hyprland, else `$HYPRLAND_CONFIG`, else the first
   `hypr/hyprland.lua`, then `hypr/hyprland.conf`, in `$XDG_CONFIG_HOME`
   (`~/.config`) and `$XDG_CONFIG_DIRS`.

hyprtilt reads the command line and the environment of the running
Hyprland process from `/proc`, so it finds the file Hyprland actually
loaded. `hyprtilt list` and `hyprtilt doctor` say which file and why.

The language comes from the file name (`.lua` is Lua, anything else
hyprlang) unless `--backend` or `backend` in `config.toml` says otherwise.
hyprtilt refuses to write a file in a language the running Hyprland does
not read.

## The managed block

```lua
-- BEGIN hyprtilt (managed)
hl.monitor({ output = "DP-1", mode = "2560x1440@179.95", position = "3360x975", scale = 1, vrr = 2 })
-- END hyprtilt
```

- hyprtilt changes only the lines between the markers (`#` instead of `--`
  in hyprlang files). Everything outside stays byte for byte the same,
  including line endings and a missing final newline.
- A rule that did not change keeps its text, spacing and comments. A
  changed rule is rewritten in a fixed format; a removed rule takes the
  comment lines directly above it along.
- Writing a layout that is already in the file changes nothing: the file is
  not written and its modification time stays.
- Inside the block, rules must be literal values. Variables, function calls
  or conditions there are refused with the line number; outside the block
  they are fine and are never touched.
- The block goes after the last monitor rule outside it, so that its rules
  come later and win. In a Lua file without such rules it goes before the
  last top-level `return`, else at the end.

### Adopting and unmanaging

`hyprtilt adopt` (or ++o++ in the interface) moves rules outside the block
into it, keeping their text. `hyprtilt unmanage` removes the markers and
leaves the rules as ordinary configuration. Rules that stay outside the
block still count: Hyprland merges or replaces rules for the same output,
and `doctor` warns about any that would override the block.

## Lua

- One `hl.monitor({...})` call per rule, keys in a fixed order: `output`,
  `mode`, `position`, `scale`, `transform`, then the others.
- A whole scale is written as a number (`scale = 1`), any other as a string
  (`scale = "1.25"`): Hyprland reads the scale as a string, and a string
  leaves no doubt about how a Lua float would be converted.
- Hyprland merges Lua rules for the same `output` string: later keys
  override earlier ones, keys left out are inherited. hyprtilt accounts for
  that when it reads the rules outside the block, and a live change always
  sends every key it owns.
- Before writing, hyprtilt compiles the new file with `luac -p` from Lua 5.5
  or 5.4 when one is installed, and refuses a write that would turn a file
  that compiled into one that does not.

## hyprlang

- One `monitor = SEL, MODE, POSITION, SCALE[, key, value]...` line per rule,
  or `monitor = SEL, disable`.
- A `monitorv2 { }` block only for fields the line form cannot express
  (`sdr_eotf`, `supports_wide_color`, `supports_hdr`, the luminances,
  `reserved`), and only for Hyprland versions that know it.
- The short forms `monitor = SEL, transform, N` and `SEL, addreserved, ...`
  are never written: they patch an earlier rule, so their meaning depends
  on where they are.
- A hyprlang rule replaces the whole earlier rule for the same selector,
  and `monitorv2` blocks are applied after every `monitor=` line. A
  `monitorv2` block outside the block therefore wins over hyprtilt's
  `monitor=` line; `doctor` reports it as a shadowing rule.

Hyprland 0.56.1 deprecates hyprlang in favour of Lua; `doctor` mentions it.

## Caelestia

For Caelestia's Lua configuration the target is `hypr-user.lua`, which
ends with `return { ... }`. The block always goes before that `return`,
because Lua refuses code after it. Caelestia's own fallback rule
(`output = ""`) stays in its files and only matters for monitors without a
rule.

## Backups

Before every write, the previous content is copied to
`<file>.bak.<UTC time>` next to the file (or into `backup.dir`), and only
the newest 10 backups of a file are kept (`backup.keep`). The new content
is written to a temporary file and renamed over the original, so the file
is never half-written; permissions are kept and symbolic links are
followed. A file that changed on disk since hyprtilt read it is not
overwritten. Files in `/nix/store` or without write permission are refused
with an explanation.

## Settings

`$XDG_CONFIG_HOME/hyprtilt/config.toml` (usually
`~/.config/hyprtilt/config.toml`). Every key is optional; unknown keys are
an error.

```toml
# The file to edit instead of the one hyprtilt finds. ~ is expanded.
target = "~/.config/caelestia/hypr-user.lua"
# Its language, when the file name misleads: "lua" or "hyprlang".
backend = "lua"
# Seconds to confirm a change before it is rolled back; 0 keeps changes
# after verification without asking.
confirm_timeout = 15
# Snapping to neighbouring edges in the interface.
snap = true
# New rules select monitors by description ("desc:...") instead of
# connector name, when the description is unique.
use_descriptions = false

[backup]
# Directory for backups; next to the file when unset.
dir = "~/.local/state/hyprtilt/backups"
# Backups kept per file; 0 disables backups.
keep = 10
```
