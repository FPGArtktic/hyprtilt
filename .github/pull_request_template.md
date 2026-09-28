<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

## Summary

<!--
What does this change do, and why is it needed? Link the issue it
resolves, for example "Fixes #123". Security fixes are not discussed in
public pull requests; see SECURITY.md.
-->

## Testing

<!--
How did you test the change? Name the tests you added or changed, and
describe any manual check on a real Hyprland session (Hyprland version,
Lua or hyprlang configuration).
-->

## Checklist

<!-- Details: https://github.com/FPGArtktic/hyprtilt/blob/main/CONTRIBUTING.md -->

- [ ] Every commit subject is `area: imperative description` with an area
      from CONTRIBUTING.md, at most 72 characters, no trailing period.
- [ ] Every commit is one logical change, explains why in its body and
      carries a `Signed-off-by:` line (`git commit -s`), certifying the
      [Developer Certificate of Origin](https://github.com/FPGArtktic/hyprtilt/blob/main/DCO).
- [ ] `just podman ci commits` passes.
- [ ] New behaviour has tests; tests never touch a real Hyprland session.
- [ ] Hyprland API facts the change relies on are in
      `docs/hyprland-lua-api.md`, with links to the sources.
- [ ] README and documentation describe any changed behaviour, command,
      exit code or file format.
