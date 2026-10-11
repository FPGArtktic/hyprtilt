<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Changelog

All notable changes to hyprtilt are documented in this file. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-10-11

### Added

- Monitors that are off keep their place on the layout canvas, hatched
  and grey, with their name, `disabled` and the size they would have, so
  they can be selected with the keyboard or the mouse and switched back
  on with `e`. A monitor disabled in the configuration file is no longer
  invisible, and a file that disables every monitor no longer draws an
  empty canvas.
- A monitor Hyprland never arranged, which has no position of its own, is
  drawn beside the layout with the tops aligned, and enabling it writes
  exactly that position: the box does not move, no monitor overlaps
  another and no gap opens. `hyprtilt enable OUTPUT` places it the same
  way.

### Changed

- The details panel shows the position of a monitor that is off and marks
  its state with the key that changes it, the footer offers `e` while
  such a monitor is selected, and moving or aligning one says why it does
  not move.
- The documentation for installing hyprtilt no longer offers
  `cargo install`; the packages the project tests and ships are the AUR
  recipes, the `.deb`, the `.rpm` and the release archives.

## [0.1.0] - 2026-09-28

### Added

- Terminal interface: monitors drawn to scale, keyboard-first editing
  (move with snapping, align, rotate, flip, mode, refresh rate, scale, VRR,
  enable and disable), undo and redo, mouse selection and dragging, a
  details panel with every refresh rate of the current resolution,
  profiles, adoption of existing rules and hotplug.
- Live apply and file writes that are verified against Hyprland's monitor
  list and rolled back unless confirmed within 15 seconds; signals roll a
  pending change back.
- Command line: `list`, `rotate`, `move`, `scale`, `mode`, `refresh`,
  `enable`, `disable`, `apply`, `save`, `adopt`, `unmanage`, `profile`,
  `doctor` and `completions`, with `--json`, `--dry-run`, `--live` and
  stable exit codes.
- Managed blocks in Lua (`hl.monitor`) and hyprlang (`monitor=`,
  `monitorv2`) files, edited in place with everything outside the block
  kept byte for byte; unchanged layouts write nothing.
- Caelestia preset: `hypr-user.lua` is found through the main
  configuration, and the block goes before its `return`.
- Backups before every write, atomic replacement, and a syntax check with
  `luac` when it is installed.
- `doctor`: whether Hyprland loads the file, rules that override the block,
  configuration errors, layout problems.
- Profiles as TOML files.
- Packages: AUR recipes `hyprtilt` and `hyprtilt-git`, `.deb`, `.rpm`,
  static release archives with signed checksums.
