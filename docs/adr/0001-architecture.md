<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# ADR 0001: Architecture of hyprtilt

- **Status:** proposed, 2026-09-28
- **Deciders:** Mateusz Okulanis (maintainer)
- **Facts:** every statement about Hyprland below is backed by
  [`docs/hyprland-lua-api.md`](../hyprland-lua-api.md), which cites the
  Hyprland sources at v0.56.2. Section numbers in the form *API §n* point
  there.

## Context

Hyprland 0.55.0 introduced a Lua configuration (`hl.monitor({...})`), and
0.56.0 made it the default for new installations. Hyprland's `main` branch
has since removed the legacy hyprlang configuration entirely (commit
`a9902ea6`, not yet in a release), so the next release will read Lua only.
Distributions with older Hyprland (Ubuntu, Debian) still use hyprlang
(`monitor=...`).

Existing monitor tools either generate their own file and add an include to
the user's main configuration (hyprmoncfg, hyprmon, nwg-displays) or do not
touch the configuration at all (wlr-randr, kanshi). None of them edits the
user's own file in place, none takes over existing `hl.monitor` calls, and
none handles dotfile frameworks such as Caelestia, which overwrite the main
configuration on update and give users a separate file
(`~/.config/caelestia/hypr-user.lua`) instead.

hyprtilt is **config-file-first**: the user's configuration file is the
source of truth. hyprtilt edits it in place, inside a managed block, with no
sidecar file, no daemon and no include lines added to files that a framework
overwrites.

### Facts that shape the design

These come from the research, and several contradict common belief (API
§1.5, §5, §11 and "Common misconceptions and wiki disagreements"):

1. **Rules for the same `output` string merge.** A later `hl.monitor` call
   with the identical `output` string starts from the earlier rule and only
   overwrites the keys it passes; the merged rule moves to the end of the
   list. Legacy `monitor=` replaces the rule wholesale instead.
2. **The last matching rule wins**, not the most specific one. A rule with
   `output = ""` is used only when no other rule matches. `DP-1` and
   `desc:Samsung ...` are different rules; whichever comes later wins.
3. **Hyprland sets `package.path`** (`<configdir>/?.lua;<configdir>/?/init.lua`
   in front of the default) and **creates a fresh `lua_State` on every
   reload**, clearing `package.loaded` for non-standard modules. The claims to
   the contrary in hyprmoncfg's code are false for 0.55.0–0.56.2 and `main`.
4. **A syntax error in a required module does not protect the old rules.**
   The reload syntax-checks only the main file; then it clears all monitor
   rules and runs the configuration. If `hypr-user.lua` fails to compile, the
   error is recorded and every monitor falls back to the `output = ""` rule.
   A broken write by hyprtilt would therefore rearrange the user's screens.
5. **Code after a top-level `return` is a Lua syntax error.** Caelestia's
   `hypr-user.lua` ends with `return { ... }` (the value is ignored), so a
   block appended at the end of that file breaks it (fact 4).
6. Hyprland runs **PUC Lua 5.5**. Integer keys (`transform`, `vrr`,
   `bitdepth`, ...) accept only integer literals: `vrr = 2`, never `2.0` or
   `"2"`.
7. Hyprland **snaps scales** to the nearest multiple of 1/120 for which the
   logical size is integral, with a notification when the requested scale was
   not close. The logical size is `round(transformed_pixels / scale)`,
   rounding half away from zero.
8. Hyprland **never fixes overlaps or gaps**; it only reports the first
   overlap in a notification.
9. `hyprctl monitors -j` reports the untransformed mode size, the effective
   scale and logical positions, but no logical size; `mirrorOf` is a numeric
   monitor ID. `configerrors -j` returns `[""]` when there are no errors.
10. No IPC request lists the files a configuration loaded. Hyprland watches
    every loaded file with inotify, and those watches are visible in
    `/proc/<pid>/fdinfo`.

## Decision drivers

The acceptance criteria of the project brief, in short: managed block in any
file with everything else preserved byte for byte; idempotent writes with a
`--dry-run` diff; taking over existing rules (`adopt`, `unmanage`);
configurable target file with a Caelestia preset and a check that Hyprland
really loads it; apply with verification and automatic rollback; keyboard-
first TUI; per-output CLI for keybindings; one static binary starting in
under 50 ms; AUR and Debian packages; repository quality on par with
`lazysubmodules`.

## Decisions

### D1. Language: Rust

Rust with ratatui + crossterm (TUI), clap (CLI, man page and completions),
serde (JSON, TOML). Go with Bubble Tea would also work; both competitors use
it. Rust wins on the criteria that matter here: a static musl binary with no
runtime, fast startup, and a type system that makes the parser and the
apply state machine hard to get wrong. No reason was found to deviate from
the brief's default.

### D2. Crates and modules

```
crates/hyprtilt-core   #![forbid(unsafe_code)] #![deny(missing_docs)]
  model        MonitorRule and its fields, OutputSelector, Layout
  geometry     transforms, logical size, scale snapping, reflow, snapping,
               alignment, overlap and gap detection
  lua          lexer (Lua 5.4/5.5), rule recognizer, managed block, generator
  hyprlang     line parser for monitor= and monitorv2 {}, managed block,
               generator
  block        the language-independent managed-block text engine
  config       target discovery (Hyprland's own order), backends, presets
  fsio         atomic writes through symlinks, backups
  ipc          HyprlandIpc trait, socket implementation, mock, JSON types
  apply        apply/verify/rollback state machine (time is an input)
  profile      profiles in TOML
  settings     application settings in TOML
  doctor       checks as data (the binary only prints them)
crates/hyprtilt        the binary: cli/ and tui/, a thin layer over core
```

`hyprtilt-core` never touches a terminal, the wall clock or the process
environment directly: time, IPC and paths are passed in. This keeps every
behaviour testable without Hyprland.

### D3. The managed block

**Markers.** Lua: `-- BEGIN hyprtilt (managed)` and `-- END hyprtilt`.
hyprlang: `# BEGIN hyprtilt (managed)` and `# END hyprtilt`. A marker is a
line whose content, without surrounding whitespace, equals the marker text.
A file with more than one block, an unterminated block or markers in the
wrong order is refused with the line numbers of the markers.

**Byte preservation.** The file is split into prefix, block and suffix by
byte offsets. Only the block is regenerated; prefix and suffix are copied
verbatim. Line endings (`\n` or `\r\n`) follow the file's first line ending,
and a missing final newline stays missing. Tests compare the prefix and
suffix bytes before and after every write.

**Content of the block.** In Lua, only `hl.monitor` calls with literal
arguments, comments and blank lines. In hyprlang, only `monitor=` lines,
`monitorv2 { }` blocks with literal values, comments and blank lines.
Anything else (a variable, a function call, `$var`) makes hyprtilt refuse to
write, naming the line; it never guesses.

**Idempotence and minimal diffs.** The block is parsed into items (rule,
comment, blank line) that keep their original text. When saving, a rule
whose parsed value is unchanged keeps its original text, including the
user's column alignment; only changed rules are regenerated, new rules are
appended, and removed rules are dropped together with the comment lines
directly above them. If nothing changed, the file is not written at all, so
saving an unchanged configuration produces no diff and no backup. Generated
rules are a fixed point: generate → parse → generate yields the same bytes.

**Placement of a new block** (first `save` or `adopt`):

1. at the position of the last `hl.monitor` / `monitor=` rule taken over by
   `adopt`, so that the block keeps its place in the evaluation order;
2. otherwise after the last monitor rule that stays outside the block, so
   that the block's rules come later and win (fact 2);
3. otherwise, in Lua, right before the last top-level `return` statement
   (fact 5), found with the lexer and a block-depth count, never with a
   regular expression;
4. otherwise at the end of the file.

Insertion points are always statement boundaries.

**Explicit core fields.** Every rule hyprtilt generates writes `mode`
(always with `@refresh`), `position` and `scale` explicitly. The defaults of
omitted fields changed between releases (Hyprland 0.55.x used 1280×720 at
`0x0`; 0.56.0 switched to preferred and auto, commit `049595e1`), and a
missing refresh rate means 60 Hz rather than the best rate (API §12).

**Ownership of fields.** Because same-string Lua rules merge (fact 1), a
block rule would inherit keys from an earlier call for the same `output`
string outside the block. hyprtilt therefore scans the whole target file for
other rules with the same selector string; `save` and `doctor` report them
and the TUI offers to adopt them. Rules in other files cannot be seen
statically; `doctor` detects their effect by comparing the live state with
the block.

**Selectors.** Rules keep the selector the user wrote (`HDMI-A-1` or
`desc:...`). New rules use the connector name; profiles may use `desc:` to
survive connector renames. `doctor` warns when a connector-name rule and a
`desc:` rule target the same monitor (fact 2).

**`unmanage`** removes the two marker lines and leaves the rules as ordinary
configuration.

### D4. Parsing Lua without executing it

A complete Lua 5.4/5.5 lexer (strings, long brackets, comments, numbers with
Lua's integer/float subtypes) feeds a recognizer that finds:

- `hl.monitor(<table>)` and `hl.monitor<table>` (call without parentheses),
  where the table contains only literals: strings, numbers (with unary
  minus), booleans, `nil` and nested tables of literals (`reserved`);
- the block depth (`function`, `if`, `do`, `repeat` open; `end`, `until`
  close; `while` and `for` open through their `do`), to find top-level
  `return` statements and to know whether a rule is conditional.

Everything else in the file is opaque text. Calls through aliases
(`local m = hl.monitor`) are not recognised; this is a documented limitation
that `doctor` covers by comparing live state.

Numbers are written the way Hyprland needs them (fact 6): integer keys as
integer literals, `scale` as the shortest decimal that round-trips, and
positions as `"XxY"` strings.

**Syntax safety net.** Before replacing a Lua file, hyprtilt compiles the
new content with `luac -p` when a Lua 5.5 compiler is available (`luac5.5`
or `luac` reporting 5.5, which is a dependency of Hyprland on Arch). If the
old file compiled and the new one does not, the write is refused. Without a
compiler the structural guarantees above still hold; `doctor` reports that
the extra check is unavailable.

### D5. hyprlang

The parser reads `monitor = SEL, MODE, POS, SCALE[, key, value]...` (fields
split on unescaped commas and trimmed, `\` line continuations joined), the
`monitor = SEL, disable|disabled`, `SEL, transform, N` and
`SEL, addreserved, T, B, L, R` sub-forms, and `monitorv2 { output = ... }`
blocks. Comments follow hyprlang rules (`#` starts a comment, `##` is a
literal `#`). Inside the block, `$variables`, `{{arithmetic}}`, `source=` and
`# hyprlang if` directives are refused; outside it they are opaque text.

The generator writes one `monitor=` line per rule, always with an explicit
`@refresh`, and never the `transform` or `addreserved` sub-forms (they only
patch an existing rule and are order-sensitive). `monitor=` accepts only
eight extra keys (`mirror`, `bitdepth`, `cm`, `sdrsaturation`,
`sdrbrightness`, `transform`, `vrr`, `icc`); an unknown key drops the whole
line. A rule that needs any other field (`sdr_eotf`, `supports_*`,
`*_luminance`, `reserved`) is written as a `monitorv2` block instead, which
requires Hyprland ≥ 0.50.0 (`sdr_eotf` ≥ 0.53.3, `icc` ≥ 0.55.0); older
versions get an error naming the field. Because `monitorv2` rules are applied
after all `monitor=` lines regardless of file order, hyprtilt never writes
both forms for the same selector.

hyprlang support has a limited lifetime (Hyprland `main` removed it), so it
gets the same correctness guarantees but no hyprlang-only features. It stays
important for now: Ubuntu 26.04 LTS ships Hyprland 0.53.3, Debian stable has
none outside backports (0.55.2), so hyprlang is the default there.

### D6. Target file and backend

The target is resolved in this order:

1. `--file` (with `--backend`, or the backend from the file extension);
2. `target` in `~/.config/hyprtilt/config.toml`;
3. the **Caelestia preset**: if Hyprland's main configuration is Caelestia's
   (it `require`s `hypr-user` and extends `package.path` with
   `~/.config/caelestia`), the target is the file that `require("hypr-user")`
   resolves to, normally `~/.config/caelestia/hypr-user.lua`;
4. Hyprland's own main configuration, found the way Hyprland finds it:
   `--config` from `/proc/<pid>/cmdline`, `HYPRLAND_CONFIG` from
   `/proc/<pid>/environ`, then `hypr/hyprland.lua` and `hypr/hyprland.conf`
   in the XDG configuration directories (`.lua` wins).

The backend follows the extension (`.lua` → Lua, anything else → hyprlang),
as in Hyprland, which creates exactly one configuration manager. The running
compositor confirms it: `hyprctl status -j` reports `configProvider` (`lua`
or `hyprlang`) since 0.55.0; older versions are hyprlang by definition. The
Lua backend requires Hyprland ≥ 0.55.0. A hyprlang target is refused when the
compositor runs a Lua configuration, because the file would never be read
(`hyprland.lua` anywhere in the XDG search path wins over any
`hyprland.conf`).

**Is the file loaded?** `doctor` combines three signals: the target's inode
appears in the inotify watches of the Hyprland process (fact 10); no
`configerrors` line points into the target file; and the live state matches
the block after a reload. It also warns about files that would shadow the
target in `require` resolution (for example `~/.config/hypr/hypr-user.lua`).

### D7. Live apply, verification and rollback

There are two ways to change the running layout, and hyprtilt uses both
(API §2):

| | Lua configuration (≥ 0.55) | hyprlang configuration |
|---|---|---|
| **Live change** (TUI `a`, CLI `--live`) | one `eval` request with the complete `hl.monitor({...})` calls of the changed outputs | one `keyword monitor SEL,...` request per changed output (`monitor=` form; `monitorv2` via `keyword` only takes effect on reload) |
| **Persistent change** (TUI `w`, CLI default) | write the block, then `reload` | write the block, then `reload` |
| **Rollback of a live change** | `reload` when the baseline is the file on disk, otherwise `eval` of the previous rules | `reload`, or `keyword` of the previous rules |
| **Rollback of a persistent change** | restore the previous file content, then `reload` | same |

Properties this relies on:

- `eval` runs `hl.monitor` in the configuration's own Lua state; the rule is
  merged into the rule with the same selector string and moved to the end,
  so it wins. The reply (`ok`) comes **before** the layout changes: the
  monitors are re-applied on the next rendered frame. hyprtilt therefore
  always verifies by polling (D8) and never trusts the reply.
- Live changes always send **complete** rules for the affected outputs (every
  field hyprtilt owns), so that merging cannot leave stale fields behind.
- `reload` runs synchronously, recreates the Lua state and drops every rule
  set through `eval` or `keyword`: it is an exact rollback to the file.
- `eval` clears the list that `configerrors` reports. hyprtilt reads
  `configerrors` only after its own `reload`, never after an `eval`.
- `eval` code never goes into a `[[BATCH]]` request: at 0.56 the batch
  splitter cuts at every `;` outside brackets.
- Hyprland's file watcher also reloads after a write, but whether it notices
  an atomic rename is not guaranteed (API §10), so hyprtilt reloads
  explicitly. A double reload is harmless.
- Before 0.55 there is no `eval`; only hyprlang exists there, so `keyword`
  covers it.

The TUI keeps a baseline (the rules in the file when it started, which
`doctor` checks against the live state) and the last confirmed live rules.
`w` writes the confirmed rules; quitting with unsaved live changes asks
whether to save, keep them until the next reload, or revert.

### D8. The apply state machine

`apply::Session` is a pure state machine. Inputs are events (`Start`,
`IpcDone(result)`, `Observed(snapshot)`, `Tick(now)`, `Confirm`, `Cancel`,
`Signal`); outputs are commands for the caller (`ApplyRules`,
`QueryMonitors`, `Restore`, `Finish(outcome)`). Time is a `Duration` since
the session started, passed in by the caller, so tests drive the countdown
without sleeping.

Phases: `Applying → Verifying → Confirming(deadline) → Confirmed`, or
`→ RollingBack(reason) → RolledBack(reason)`. Verification polls the monitor
list every 100 ms for up to 3 s and compares the expected layout (effective
scale, logical position, transform, mode within 1 px and 1 Hz, enabled
state) with the observed one. A timeout, a failed verification, `Cancel`,
`SIGINT` or `SIGTERM` leads to rollback. The default confirmation timeout is
15 s; 0 disables the confirmation (but not the verification).

### D9. IPC

The `HyprlandIpc` trait has one required method, a raw request returning the
raw reply; typed helpers (`monitors`, `version`, `status`, `config_errors`,
`reload`, `eval`, `keyword`) parse on top of it. This keeps the mock trivial
(a table of request → reply, recording the requests) and lets tests use
replies captured from a real session.

The socket implementation talks to
`$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock` directly
(`hyprctl` is not required) and follows the wire format exactly (API §3):

- every request carries a flag prefix (`j/monitors all`, `/eval ...`), so a
  `/` inside Lua code is never taken for a flag separator;
- the request is sent with a single write followed by `shutdown(SHUT_WR)`:
  Hyprland 0.56 reads in 1023-byte chunks without a timeout, and a request
  whose length is a multiple of 1023 would otherwise block its event loop;
- the reply is read to EOF with a timeout.

JSON is parsed leniently (unknown fields ignored), since `main` already adds
fields. `monitors all` is used, never `monitors`, so disabled and mirrored
outputs are visible; `mirrorOf` (a numeric ID) is resolved to a name; the
`vrr` field is the current adaptive-sync state, not the configured mode, so
the configured value always comes from the file; `configerrors` `[""]` means
no errors.

Events come from `.socket2.sock`, read continuously on a dedicated thread
(Hyprland drops a client after 64 unread events). `monitoradded(v2)`,
`monitorremoved(v2)` and `configreloaded` trigger a refresh of the monitor
list. Mode, position, scale and transform changes produce no event, which is
why verification polls.

### D10. Geometry

- Logical size: swap width and height for odd transforms, divide by the
  effective scale, round half away from zero (`f64::round`), exactly as
  Hyprland does (fact 7).
- Scale: hyprtilt runs Hyprland's snapping algorithm (1/120 grid, `f32`
  storage, `f64` division, search up before down) and shows the effective
  scale before writing; it writes the snapped value so that Hyprland never
  shows its warning.
- Transform: 0–7 as in `wl_output_transform`. `r` maps
  `t → (t & 4) | ((t + 1) & 3)`, `R` the inverse, `f` toggles bit 2. The
  physical direction of transform 1 is confirmed on real hardware before
  release (API §13).
- Positions are always written explicitly (`"XxY"`), never `auto`, because
  Hyprland's automatic placement depends on connection order. `auto` values
  read from a file are kept until the user moves the monitor.
- Reflow after rotation or scale change: monitors that touched the changed
  monitor's right edge move by the width difference, monitors that touched
  its bottom edge move by the height difference, transitively. Nothing else
  moves.
- Snapping (toggle) aligns edges to neighbours' edges within a threshold;
  alignment shortcuts align bottom, top or vertical centre to the nearest
  neighbour. Overlaps are detected and block `apply` and `save` (fact 8);
  gaps are shown as warnings.
- Disabled and mirrored monitors take no part in layout checks.

### D11. Profiles and settings

Profiles are TOML files in `~/.config/hyprtilt/profiles/<name>.toml`: a list
of monitor rules plus optional backend and target. TOML is human-editable,
allows comments, and is the Rust ecosystem's configuration format; JSON has
no comments and Lua would need an interpreter. Application settings live in
`~/.config/hyprtilt/config.toml` (target, backend, backup directory and
count, confirmation timeout, snapping default).

### D12. Writing files

- Symbolic links are followed and the **target of the link** is replaced
  (dotfile managers such as GNU Stow rely on links staying links).
- Writes are atomic: temporary file in the same directory, `fsync`, rename,
  `fsync` of the directory. Permissions of the original are kept.
- Every write is preceded by a backup `<file>.bak.<YYYYMMDDTHHMMSSZ>` (or in
  the configured backup directory); the number of kept backups is
  configurable (default 10).
- After a write, hyprtilt runs `hyprctl reload` explicitly instead of
  relying on Hyprland's file watcher, whose reaction to a rename is not
  guaranteed (API §10).

### D13. Command line

Subcommands as in the brief: `list`, `rotate`, `move`, `scale`, `mode`,
`enable`, `disable`, `apply`, `save`, `adopt`, `unmanage`, `profile`,
`doctor`, `completions`, `man`. The per-output commands change the managed
block and reload, which is what a keybinding needs; `--live` changes only the
running session. Every state-changing command supports `--dry-run` (prints
the file diff or the IPC requests) and `--json` where it prints data. Exit
codes:

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | generic error |
| 2 | usage error (clap) |
| 3 | Hyprland not running or IPC failed |
| 4 | configuration file cannot be parsed or is refused (unknown content in the block, several blocks) |
| 5 | the requested output does not exist |
| 6 | the change was rolled back (not confirmed, or verification failed) |
| 7 | `doctor` found problems |

### D14. Terminal interface

The TUI keeps an `App` state and a pure `render(&App, &mut Frame)`; input is
mapped to actions, actions change the state, and side effects (IPC, file
writes) go through the core. This keeps the TUI testable with
`ratatui::backend::TestBackend`. Layout at 80×24: canvas on the left,
details panel on the right (below the canvas when narrower than 80 columns),
status line at the bottom. Keys follow the brief (hjkl and arrows, `Tab`,
`r`/`R`/`f`, `m`/`s`/`v`/`e`, `a`, `w`, `u`, `?`, `q`); mouse is optional.

### D15. Dependencies

Only what the design needs: `serde`, `serde_json`, `toml`, `thiserror`,
`similar` (diffs) in core; `clap` (+ `clap_complete`, `clap_mangen`),
`ratatui`, `crossterm`, `signal-hook` in the binary. No async runtime: the
IPC is a few short blocking requests plus one event thread. `cargo-deny`
enforces licenses compatible with GPL-3.0-only.

### D16. Build, CI and release

- One build image (Arch Linux, pinned by digest and archive date) runs every
  `just` target locally (`just podman <target>`) and in CI (`container:`),
  published to `ghcr.io/fpgartktic/hyprtilt-build`.
- **Release pipeline: a hand-written `release.yml`, not cargo-dist.**
  cargo-dist generates its own workflow and installers; hyprtilt needs
  cosign signatures, an SBOM, `.deb` and `.rpm`, a vendor tarball, man pages,
  completions and an AUR job, most of which would be custom steps anyway.
  A hand-written workflow that calls the same `just` targets as developers
  keeps one source of truth.
- Coverage: `cargo-llvm-cov` with thresholds enforced in CI itself
  (`hyprtilt-core` ≥ 85 %, workspace ≥ 70 %), independent of any external
  service; the report is uploaded to Codecov when `CODECOV_TOKEN` is set.
- Artifacts are signed keyless with cosign (GitHub OIDC); SBOM by syft;
  changelog by git-cliff.

### D17. Distribution: which AUR packages

| Package | Builds from | Needs | When |
|---|---|---|---|
| `hyprtilt-git` | `main` of this repository | Rust toolchain on the user's machine | now; the only package possible before a release |
| `hyprtilt-bin` | the signed static musl tarball of a release | nothing but `curl` and a checksum | from v0.1.0 |
| `hyprtilt` | the source tarball of a release | Rust toolchain | from v0.1.0 |

`-git` is the right first package: there is no release yet, and it follows
`main`. A `-bin` package is worth adding with the first release: the binary is
static, so the package is small, installs in seconds without a Rust
toolchain, and uses the exact bytes that were signed and checksummed in CI.
The source package `hyprtilt` remains the canonical AUR package and the one
the brief's automated AUR job updates. All three `provide`/`conflict` on
`hyprtilt`. Publishing any of them needs the maintainer's explicit consent.

Debian/Ubuntu get a `.deb` from `cargo-deb` (static binary, so it installs on
older LTS releases), Fedora an `.rpm` from `cargo-generate-rpm`.

### D18. Licence and MSRV

GPL-3.0-only, as in `lazysubmodules`. MSRV 1.88 (edition 2024, let-chains);
checked in CI with the MSRV toolchain from the build image.

## Non-goals for 0.x

A hotplug or lid daemon, workspace planning, integrations with panels
(Omarchy), other compositors, packages beyond Arch, Debian/Ubuntu, RPM and
cargo.

## Consequences

- hyprtilt must track Hyprland's rule semantics (merging, precedence, scale
  snapping) closely; `docs/hyprland-lua-api.md` is updated with every
  Hyprland release that touches them.
- Users who keep monitor rules in several files get warnings rather than
  automatic handling.
- The hyprlang backend will become legacy with the next Hyprland release.

## Open questions

See API §13 for facts that need a runtime check on real hardware.
