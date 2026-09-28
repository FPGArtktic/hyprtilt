---
description: Every hyprtilt command, its options, JSON output and exit codes.
---

<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# The command line

```text
hyprtilt [OPTIONS] [COMMAND]
```

Without a command, hyprtilt opens [the terminal interface](tui.md).
`hyprtilt --help`, `hyprtilt <command> --help` and `man hyprtilt` show the
same text as this page, generated from one definition.

## Global options

| Option | Meaning |
|---|---|
| `-f`, `--file PATH` | edit this file instead of the one hyprtilt finds ([how it finds one](configuration.md#which-file)) |
| `--backend lua\|hyprlang` | the file's language, instead of guessing it from the file name |
| `-n`, `--dry-run` | print the diff of the file and the IPC requests; change nothing |
| `--json` | print the result as JSON on standard output |

Options may come before or after the command.

## Per-output commands

For keybindings and scripts. Each one changes one output in the managed
block, writes the file (with a backup), reloads Hyprland and checks that
Hyprland shows the change. If it does not, the file is restored, Hyprland
reloads it, and the command exits with code 6. There is no confirmation
prompt.

| Command | Argument |
|---|---|
| `rotate OUTPUT ANGLE` | `90`, `180`, `270` or `-90`: quarter turns added to the transform (the flip is kept) |
| `move OUTPUT XxY` | the new top-left corner in logical pixels, such as `1440x1335` |
| `scale OUTPUT FACTOR` | a number, snapped the way Hyprland snaps it, or `auto` |
| `mode OUTPUT MODE` | `WxH@Hz`, `preferred`, `highres` or `highrr` |
| `refresh OUTPUT RATE` | Hz (the nearest rate the monitor offers within 1 Hz), `max`, `min`, `up` or `down` |
| `enable OUTPUT` | enable the output |
| `disable OUTPUT` | disable the output |

`OUTPUT` is a connector name (`DP-1`), the selector of its rule
(`desc:...`), or the beginning of its description.

`--live` changes only the running session and leaves the file alone; the
change is lost on the next reload. If Hyprland does not show it, the file's
state is reloaded.

Rotating, or changing the mode or the scale, changes the output's logical
size; outputs that touched its right or bottom edge move with it, and the
command lists them. Changes that would make outputs overlap are refused
(exit code 4).

`refresh` keeps the resolution and picks the rate the monitor lists for it.
When no listed rate is within 1 Hz, hyprtilt warns that Hyprland will try
a custom mode, and verification decides whether it is kept:

```console
$ hyprtilt refresh DP-1 max
DP-1: mode 2560x1440@164.98 -> 2560x1440@179.95
wrote /home/you/.config/caelestia/hypr-user.lua (backup: /home/you/.config/caelestia/hypr-user.lua.bak.20260928T170542Z)
$ hyprtilt refresh DP-1 down --live
DP-1: mode 2560x1440@179.95 -> 2560x1440@164.98
applied to the running session (until the next reload)
```

## `list`

The live state of every monitor from Hyprland and the rules of the block,
with the line of each rule and the differences between them. Rules outside
the block that `adopt` can take are listed at the end. Without Hyprland,
the outputs come from the block.

## `apply`

```text
hyprtilt apply [--profile NAME] [--confirm-timeout SECONDS | --no-confirm]
```

Without `--profile`: reload the file as it is, check that Hyprland shows
the block, and ask on the terminal whether to keep it. Without `y` within
the timeout (15 s by default, `confirm_timeout` in
[config.toml](configuration.md#settings)), the previous live layout is
restored. `--confirm-timeout 0` and `--no-confirm` keep it after the check.

With `--profile NAME` (or `profile apply NAME`): write the profile's rules
into the block, reload, check and ask; without confirmation the previous
file comes back. See [Applying and rollback](apply.md).

## `save`

Write the running layout into the managed block: every output gets a rule
with its current mode, position, scale, transform and state. Other fields
of existing rules (VRR, colour settings, ...) are kept. A block that already
describes the running layout is not written.

## `adopt`

Move monitor rules from outside the block into it; `--line N` (repeatable)
takes only the rule starting on line N. The rules keep their text. Rules
built from variables or function calls are not adopted; `list` says why. In
hyprlang files, a rule is not adopted across a rule that stays outside and
would then change its meaning; hyprtilt names the line.

## `unmanage`

Remove the two marker lines. The rules stay as ordinary configuration.

## `profile`

| Command | |
|---|---|
| `profile list` | the saved profiles |
| `profile save NAME [--desc]` | save the running layout; `--desc` selects monitors by description, so the profile survives renamed connectors |
| `profile apply NAME` | the same as `apply --profile NAME` |
| `profile delete NAME` | delete it |

See [Profiles](profiles.md).

## `doctor`

Check the setup and explain every problem; exits with 7 when there are
warnings or failures. See [Troubleshooting](doctor.md).

## `completions SHELL`

Print completions for `bash`, `zsh`, `fish`, `elvish` or `powershell`.

## Exit codes

Scripts may rely on these; a code never changes meaning.

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | any other error, including a rollback that failed |
| 2 | invalid command line |
| 3 | Hyprland is not running, or IPC failed |
| 4 | the configuration file cannot be parsed or the change is refused (overlaps, wrong language, read-only file) |
| 5 | no such output |
| 6 | the change was rolled back: not confirmed, or Hyprland did not show it |
| 7 | `doctor` found problems |

## JSON

With `--json`, every command prints one JSON document on standard output;
messages and warnings still go to standard error. The shapes:

`list`
:   `target` (`path`, `backend`, `reason`), `hyprland` (`version`,
    `provider`) or `null`, `block` (`begin_line`, `end_line`) or `null`,
    `outputs` (name, description, enabled, mode, refresh_rate, position,
    scale, transform, the block `rule` and its `rule_line`, `differences`
    between the rule and the live state), `detached_rules` (rules for
    outputs that are not connected), `outside_rules` (`line`, `rule`,
    `adoptable`, `reason`).

Per-output commands
:   `output`, `changes` (`"field old -> new"`), `moved` (other outputs that
    moved), `rule` (the output's rule after the change), `refresh` (`from`,
    `to`, `custom`) for `refresh`, `live`, `dry_run`, `file`, `backup`,
    `changed`, `requests` (the IPC requests sent), `verified`.

`apply`, `save`, `adopt`, `unmanage`
:   the same `file`, `backup`, `changed`, `requests`, `verified`, with
    `profile` or `lines` where they apply.

`doctor`
:   `checks` (`id`, `status`: `ok`, `info`, `warn` or `fail`, `title`,
    `detail`) and `problems`, the number of warnings and failures.

`profile list`
:   an array of names.

Rules use the field names of the Lua configuration (`output`, `mode`,
`position`, `scale`, `transform`, `vrr`, ...); fields a rule does not set
are left out.
