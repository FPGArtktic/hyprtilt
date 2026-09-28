<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# hyprtilt

A terminal UI and CLI for arranging monitors in [Hyprland](https://hypr.land)
that edits **your** configuration file in place, inside a managed block, for
both the Lua configuration (`hl.monitor({...})`) and hyprlang (`monitor=...`).
One static Rust binary, no daemon.

> **Status: under construction.** Nothing is released yet. The design is in
> [`docs/adr/`](docs/adr/) and the verified Hyprland API notes are in
> [`docs/hyprland-lua-api.md`](docs/hyprland-lua-api.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Every commit is signed off under the
[Developer Certificate of Origin](DCO).

## License

hyprtilt is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License, version 3 only (`GPL-3.0-only`), as
published by the Free Software Foundation. See [LICENSE](LICENSE) for the full
text.

## Author

Mateusz Okulanis <FPGArtktic@outlook.com>
