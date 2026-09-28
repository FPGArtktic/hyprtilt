---
description: Save monitor layouts as named profiles and apply them.
---

<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Profiles

A profile is a named set of monitor rules, for example one for the desk and
one for the laptop alone. Profiles are TOML files in
`$XDG_CONFIG_HOME/hyprtilt/profiles/` (usually
`~/.config/hyprtilt/profiles/`).

```sh
hyprtilt profile save desk          # the running layout
hyprtilt profile save desk --desc   # select monitors by description
hyprtilt profile list
hyprtilt profile apply desk         # write, reload, verify, confirm
hyprtilt profile delete desk
```

In the interface, ++p++ lists the profiles: ++enter++ loads one into the
editor (++a++ or ++w++ then applies or writes it, ++u++ undoes the load),
++s++ saves the edited layout under a new name. Monitors a loaded profile
does not mention keep their current settings there, as new rules.

Applying a profile writes its rules into the managed block, replacing the
rules there, reloads Hyprland, verifies and asks for confirmation.
Monitors the profile does not mention get no rule from the block, so rules
outside it or Hyprland's defaults decide their state.

## Format

```toml
# hyprtilt profile "desk"; the format is described in
# https://hyprtilt.readthedocs.io/en/latest/guide/profiles/

backend = "lua"

[[monitor]]
output = "HDMI-A-1"
mode = "2560x1440@144"
position = "0x0"
scale = 1.0
transform = 1

[[monitor]]
output = "desc:Samsung Electric Company Odyssey G50F SERIAL0002"
mode = "2560x1440@179.95"
position = "3360x975"
scale = 1.0
vrr = 2
```

| Key | Meaning |
|---|---|
| `backend` | optional: `lua` or `hyprlang`, the language of the file the profile is written into |
| `target` | optional: that file, when it is not the one hyprtilt finds; `~` is expanded |
| `[[monitor]]` | one table per rule |

A `[[monitor]]` table takes the keys of the Lua `hl.monitor` call:
`output` (required), `mode`, `position`, `scale`, `transform`, `disabled`,
`vrr`, `mirror`, `bitdepth`, `cm`, `sdr_eotf`, `sdrbrightness`,
`sdrsaturation`, `icc`, `supports_wide_color`, `supports_hdr`,
`sdr_min_luminance`, `sdr_max_luminance`, `min_luminance`,
`max_luminance`, `max_avg_luminance` and `reserved`. Unknown keys and two
rules for the same output are errors, so a typo cannot pass unnoticed.

Profile names may contain letters, digits, `.`, `_` and `-`, and must not
start with `.`.
