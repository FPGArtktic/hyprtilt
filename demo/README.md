<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Demo setup

An in-memory Hyprland for trying hyprtilt without a compositor, and for
recording the screenshots and the demo GIF of the documentation
(`just media`, which runs `scripts/docs-media.py`).

- `fake.json`: Hyprland 0.56.2 with a Lua configuration and three monitors
  (the maintainer's desk: a portrait 1440p monitor, a laptop panel and a
  180 Hz 1440p monitor), in the format of `hyprctl monitors all -j`.
- `hypr-user.lua`: the configuration file it loads, a Caelestia
  `hypr-user.lua` with monitor rules outside a managed block.

hyprtilt writes the file it edits, so work on a copy:

```sh
cp -r demo /tmp/hyprtilt-demo
cargo run -- --fake-hyprland /tmp/hyprtilt-demo/fake.json
```

`--fake-hyprland` is hidden from `--help`: it is meant for demonstrations
and tests, never for real use.
