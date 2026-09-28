<!-- SPDX-License-Identifier: GPL-3.0-only -->
<!-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com> -->

# Hyprland monitor configuration API (Lua and hyprlang)

Do not guess the Hyprland API; this document is the source of truth. If you
need a fact that is not here, research it from the Hyprland sources, add it
here with a link, and only then write code that depends on it.

## Scope and method

All statements describe **Hyprland v0.56.2** (tag `v0.56.2`, commit
[`efb50993780079460b0cbed1363e2166a2de1d9f`](https://github.com/hyprwm/Hyprland/tree/v0.56.2)),
checked on **2026-09-28**. Hyprland `main` as of 2026-09-27
([`4bb6844b`](https://github.com/hyprwm/Hyprland/tree/4bb6844b0351e4fbf2e3d4e46ae71b551a0e0a42))
was compared for every fact; a difference is marked **main:**, otherwise
`main` is unchanged apart from logging macros and line shifts. Other sources:
hyprland-wiki at
[`d5498a98`](https://github.com/hyprwm/hyprland-wiki/tree/d5498a9876fc00d7324d74a7c19a8e8473bad207),
Caelestia dotfiles at
[`d459e918`](https://github.com/caelestia-dots/caelestia/tree/d459e9182ae55ba7eb29fa90781297224911d6bd),
hyprlang [v0.6.8](https://github.com/hyprwm/hyprlang/tree/v0.6.8) and
hyprutils [v0.14.1](https://github.com/hyprwm/hyprutils/tree/v0.14.1) (the
build version; the runtime library is 0.14.2 with identical `string/` and
`path/` code). Runtime facts come from the maintainer's live Hyprland 0.56.2
session (Arch Linux, Caelestia) using only read-only commands
(`hyprctl version -j`, `monitors all -j`, `configerrors -j`, `getoption`,
`systeminfo`, `instances -j`, reading socket2), `/proc` inspection and local
`lua5.5`/`luac5.5` experiments; they are marked *live*. Nothing that changes
compositor state was run, so anything that would need it is marked **needs
runtime confirmation** and collected in §13. Every fact went through a
research pass and an independent adversarial verification pass; where the
verifier corrected a claim, the corrected statement is used. When the wiki and
the code disagree, the code wins and the disagreement is listed in
[Common misconceptions and wiki disagreements](#common-misconceptions-and-wiki-disagreements).
"Rule" means `Config::CMonitorRule`; "output string" means the `output` value
of `hl.monitor` or the first field of a legacy `monitor=` line.

## Summary for implementers

1. **Versions.** Lua config exists since **0.55.0**; `hyprland.lua` is
   preferred over `hyprland.conf` since 0.55.0; a fresh install generates a
   `.lua` only since 0.55.3 / 0.56.0. `main` has removed hyprlang entirely
   (untagged), and 0.56.1+ shows *".conf config format ... will be removed in
   Hyprland 0.57"*. The hyprlang backend is for Hyprland ≤ 0.56.x only (§6).
2. **Detect the backend** with `j/status` → `configProvider` (`lua` or
   `hyprlang`, 0.55+), plus which file exists. Never version-gate on "< 0.57":
   git builds of `main` report `"version": "0.56.0"` and have no hyprlang (§6, §7).
3. **`hl.monitor({...})`** takes one table; `output` is required (string, or
   a number converted to string); 22 other keys, exact and case-sensitive.
   Errors never abort the script and the rule is still added with fallbacks (§1).
4. **Types.** Integer keys (`transform`, `vrr`, `bitdepth`, `supports_*`,
   `sdr_max_luminance`, `max_luminance`, `max_avg_luminance`) accept only a
   Lua integer or boolean: write `vrr = 2`, never `2.0` or `"2"`. `scale` is a
   string key that also accepts numbers. `position` is a `"XxY"` string, never
   a table (§1.3).
5. **Same output string merges.** A later `hl.monitor` with an identical
   `output` copies the earlier rule, overwrites only the keys it passes and
   moves to the end. A managed block must write every field it owns. Legacy
   `monitor=` replaces wholesale (§1.5, §11).
6. **Last-added matching rule wins**, not the most specific; `output = ""`
   is a fallback used only when nothing matches. `DP-1` and `desc:...` for the
   same monitor are two rules (§11).
7. **`desc:`** is a trimmed prefix match against `make model serial` or the
   backend description (both comma-free). Write the full `description` from
   `hyprctl monitors -j`, serial included, no space after `desc:` (§9).
8. **Always write `mode` with `@Hz`**: a bare `"WxH"` targets 60 Hz, not the
   best rate (§12.3).
9. **Scale** is snapped at apply time to a multiple of 1/120 for which
   `pixelSize / scale` is integral on both axes; logical size is
   `round(transformed_pixels / scale)`, half away from zero (§4).
10. **Positions** are logical coordinates. Hyprland never fixes overlaps or
    gaps; it reports only the first overlap. Write explicit integer positions (§12.2).
11. **Live preview**: Lua → `hyprctl eval 'hl.monitor({...})'` (0.55+);
    hyprlang → `hyprctl keyword monitor ...` (refused under Lua). `eval`
    clears `configerrors`, applies on the next rendered frame and is lost on
    reload (§2).
12. **Reload** is synchronous and always replies `ok`. It syntax-checks only
    the main file. A syntax error in a `require`d file such as
    `hypr-user.lua` clears all rules and leaves only the `output = ""`
    fallback: validate generated Lua with a Lua 5.5 parser before writing (§2.3, §5).
13. **Top-level `return`**: the managed block must go before the last
    top-level `return` of `hypr-user.lua`; code after it is a syntax error (§5.7).
14. **Autoreload** fires on `IN_CLOSE_WRITE` of the main file and every
    `require`d file; whether an atomic rename triggers it needs runtime
    confirmation, so send an explicit `reload` after writing (§10).
15. **IPC**: `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`;
    request `"<flags>/<command>"`; one `write`, `shutdown(SHUT_WR)`, read to EOF
    immediately; never put Lua into `[[BATCH]]` (§3).
16. **`monitors all -j`** (not `monitors`) includes disabled and mirrored
    outputs; `width`/`height` are untransformed; there is no logical size;
    `mirrorOf` is a numeric ID as a string; `vrr` is the current adaptive-sync
    state. `configerrors -j` returns `[""]` when there are no errors (§3.4).
17. **socket2** has no event for mode, position, scale or transform changes;
    poll after applying. `configreloaded` is also sent after a failed reload (§3.6).
18. **No IPC request lists loaded files.** Use the inotify watches in
    `/proc/<pid>/fdinfo`, `configerrors` and a live-state comparison (§10).
19. **wlr-output-management** clients (kanshi, wlr-randr, nwg-displays)
    override config rules while their state is stored (§11.4).
20. A **VRR-only** rule change may not be applied by eval or reload, because
    rule comparison ignores `vrr` (needs runtime confirmation, §2.2).
21. **Legacy**: `monitor=` supports only 8 extra keywords; an unknown keyword
    drops the whole line. `monitorv2` (≥ 0.50) is always applied after all
    `monitor=` lines; its `sdr_eotf` needs ≥ 0.52 and `icc` ≥ 0.55 (§8).
22. **Distributions**: Ubuntu 26.04 LTS ships 0.53.3 (hyprlang only), Debian 13
    backports 0.55.2 (generates `.conf`), Arch / Tumbleweed / Debian testing /
    NixOS unstable / Ubuntu 26.10 ship 0.56.2 (§6.3).

## Common misconceptions and wiki disagreements

| Belief | What the code does | Source |
|---|---|---|
| "Hyprland does not set `package.path`" (hyprmoncfg, per the project brief) | Every Lua state gets `<configdir>/?.lua;<configdir>/?/init.lua` prepended to the default path, since 0.55.0 | [ConfigManager.cpp L534-546](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L534-L546) |
| "`require` caches, so a reload does not re-apply the layout" (hyprmoncfg, per the brief) | A successful reload creates a new `lua_State` and clears `package.loaded` for non-stdlib modules; modules run again | [ConfigManager.cpp L655-730](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L655-L730) |
| "Fundamental Lua syntax errors will make Hyprland refuse to reload" (wiki `configuring/core/_index.md` L141) | Only for the **main** file; a syntax error in a required module becomes a config error after all rules were cleared | [ConfigManager.cpp L682-701](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L682-L701), [wiki](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/configuring/core/_index.md) |
| "Each `require` is a separate Lua scope" (wiki same file L70) | One shared `lua_State` and global table; only errors are isolated (pcall in `safeLuaRequire`) | [LuaBindingsRegistration.cpp L96](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsRegistration.cpp#L96), [ConfigManager.cpp L312-377](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L312-L377) |
| "Overlapping monitors will not be registered" (wiki `monitors/positioning.md` L7-8) | Only an ERR log and a 15 s notification; the monitor stays at its position | [MonitorLayoutController.cpp L39-53](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/state/MonitorLayoutController.cpp#L39-L53), [wiki](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/configuring/core/monitors/positioning.md) |
| "An auto direction on the first output is ignored, it goes to (0,0)" (wiki `positioning.md` L28) | No caller passes `isFirst = true`; a lone `auto-left` monitor lands at `(-w, 0)` | [Parser.hpp L14](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.hpp#L14), [MonitorPositionController.cpp L16-101](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/state/MonitorPositionController.cpp#L16-L101) |
| `vrr` default is `0` (wiki `monitors/_index.md` field table L22) | An omitted key leaves `m_vrr = nullopt` = follow `misc:vrr` (whose default is 0); the wiki's own `modes.md` L54-59 lists `-1` | [MonitorRule.hpp L69](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L69), [wiki](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/configuring/core/monitors/_index.md) |
| `sdr_eotf` options are `default`/`gamma22`/`srgb` (wiki table) | Also `auto`, `gamma22force` and `"0"`-`"3"` | [TransferFunction.cpp L10-19](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/helpers/TransferFunction.cpp#L10-L19) |
| The field table lists `reserved_area` only | `reserved` is an identical alias | [LuaBindingsConfigRules.cpp L88-97](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L88-L97) |
| `bitdepth` options are 8/10 (wiki table) | Any integer or boolean is accepted; only exactly `10` enables 10-bit | [LuaBindingsConfigRules.cpp L113-117](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L113-L117) |
| "Semicolons in batch commands must be backslash-escaped" (wiki `using-hyprctl.md` L281) | v0.56.2 has no escaping; `;` inside `[ ]` is protected instead. Escaping exists only on `main` (ff24e175, untagged), which in turn dropped bracket protection | [HyprCtl.cpp L1309-1333](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L1309-L1333), [wiki](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/configuring/core/advanced-configuration/using-hyprctl.md) |
| "Unclosed connections freeze Hyprland until the five-second timeout" (wiki `ipc/_index.md` L22) | The 5 s `poll` covers only the first data; later blocking `read()`s and the reply `write()` have no timeout | [HyprCtl.cpp L2264-2281](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L2264-L2281), [wiki](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/ipc/_index.md) |
| `configreloaded` is "emitted when the config is done reloading" (wiki `ipc/_index.md` L78) | Also emitted when the reload failed the phase-1 syntax check and the old config stays | [ConfigManager.cpp L692-696](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L692-L696) |
| `hl.env("HYPRLAND_CONFIG", ...)` chooses the config file (wiki `environment-variables.md` L29) | The path is read with `getenv` before the config runs and cached; `hl.env` could matter only after `reload full-reset` (needs runtime confirmation) | [Jeremy.cpp L32-33](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/supplementary/jeremy/Jeremy.cpp#L32-L33), [LuaBindingsConfigRules.cpp L478-508](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L478-L508) |
| `hyprctl keyword monitor ...` works for live changes | Only with the legacy manager; under Lua it replies `keyword can't work with non-legacy parsers. Use eval.`; `main` has no `keyword` command | [HyprCtl.cpp L1160-1163](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L1160-L1163) |
| The `vrr` field of `monitors -j` is the configured VRR mode | It is the output's current adaptive-sync state (bool); live DP-1 with rule `vrr = 2` reports `false` | [HyprCtl.cpp L283](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L283) |
| `mode = "disable"` disables an output in Lua | It fails `parseMode` (no `x`) and leaves the mode at preferred; use `disabled = true` | [Parser.cpp L124-127](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L124-L127) |
| A socket2 client is dropped after 64 unread events | Events are written directly; only events that hit `EAGAIN` are queued, and the client is dropped when 64 are already queued | [EventManager.cpp L126-192](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/managers/EventManager.cpp#L126-L192) |
| Hyprland embeds Lua 5.4 (hyprtilt lexer commit) | The maintainer's binary links PUC Lua 5.5 (`liblua.so.5.5`, Arch lua 5.5.1); see §5.1 | [CMakeLists.txt L291](https://github.com/hyprwm/Hyprland/blob/v0.56.2/CMakeLists.txt#L291) |
| "HyprMon has no Hyprland 0.55 Lua support" (hyprmoncfg README table L249) | HyprMon's README and code document a Lua writer (at HEAD `32ad27f9`) | [hyprmon README L249-262](https://github.com/erans/hyprmon/blob/32ad27f94f40fd0dd81f982cc8208bb615607202/README.md#L249-L262) |
| The wiki's Ubuntu section lists 24.10 as an option | Launchpad marks 24.10 Obsolete (accessed 2026-09-28) | [launchpad.net/ubuntu/+series](https://launchpad.net/ubuntu/+series) |

The current wiki snapshot documents monitors only in Lua; `monitor=`,
`monitorv2` and `addreserved` appear nowhere under `content/` (only hyprpaper
and hyprlock use `monitor =`), so the hyprlang grammar in §8 rests on code
only ([wiki monitors/_index.md](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/configuring/core/monitors/_index.md)).

## 1. hl.monitor API

### 1.1 Binding and call shape

- `hl.monitor` is the C function `hlMonitor`, registered as field `monitor`
  of the global `hl` table with the `CConfigManager*` as upvalue 1
  ([LuaBindingsConfigRules.cpp L1392-1396](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L1392-L1396),
  [LuaBindingsInternal.cpp L454-458](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsInternal.cpp#L454-L458)).
- It returns no values on any path; the LuaLS stub declares
  `fun(spec: HL.MonitorSpec): nil`
  ([generateLuaStubs.py L530](https://github.com/hyprwm/Hyprland/blob/v0.56.2/meta/generateLuaStubs.py#L530);
  installed as `/usr/share/hypr/stubs/hl.meta.lua` L856, *live*). There is no
  handle and no way to read rules back from Lua.
- The whole function (the key loop, merge and error paths referred to below):

```cpp
// src/config/lua/bindings/LuaBindingsConfigRules.cpp L1103-1165 (v0.56.2)
static int hlMonitor(lua_State* L) {
    auto* self = sc<CConfigManager*>(lua_touserdata(L, lua_upvalueindex(1)));

    if (!lua_istable(L, 1)) {
        self->addError("hl.monitor: argument must be a table");
        return 0;
    }

    const std::string sourceInfo = Internal::getSourceInfo(L);

    lua_getfield(L, 1, "output");
    if (!lua_isstring(L, -1)) {
        self->addError(std::format("{}: hl.monitor: 'output' field is required and must be a string", sourceInfo));
        lua_pop(L, 1);
        return 0;
    }
    const std::string output = lua_tostring(L, -1);
    lua_pop(L, 1);

    CMonitorRuleParser parser(output);

    const auto         existing = std::ranges::find_if(Config::monitorRuleMgr()->all(), [&output](const auto& rule) { return rule.m_name == output; });
    if (existing != Config::monitorRuleMgr()->all().end())
        parser.rule() = *existing;

    lua_pushnil(L);
    while (lua_next(L, 1) != 0) {
        if (lua_type(L, -2) != LUA_TSTRING) {
            lua_pop(L, 1);
            continue;
        }

        const char* key = lua_tostring(L, -2);

        if (std::string_view{key} == "output") {
            lua_pop(L, 1);
            continue;
        }

        const auto* desc = Internal::findDescByName(MONITOR_FIELDS, key);

        if (!desc) {
            self->addError(std::format("{}: hl.monitor: unknown field '{}'", sourceInfo, key));
            lua_pop(L, 1);
            continue;
        }

        auto val = UP<ILuaConfigValue>(desc->factory());
        auto err = val->parse(L);
        if (err.errorCode != PARSE_ERROR_OK)
            self->addError(std::format("{}: hl.monitor: field '{}': {}", sourceInfo, key, err.message));
        else if (!desc->apply(val.get(), parser))
            self->addError(std::format("{}: hl.monitor: error applying field '{}'", sourceInfo, key));

        lua_pop(L, 1);
    }

    Config::monitorRuleMgr()->add(std::move(parser.rule()));

    Supplementary::refresher()->scheduleRefresh(Supplementary::REFRESH_MONITOR_STATES);

    return 0;
}
```

Source: [LuaBindingsConfigRules.cpp L1103-1165](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L1103-L1165).
**main:** identical body, at L1110.

### 1.2 Fields

The 22 non-`output` keys are the entries of `MONITOR_FIELDS`
([LuaBindingsConfigRules.cpp L81-178](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L81-L178)).
Lookup is exact, case-sensitive string equality
([LuaBindingsInternal.hpp L174-182](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsInternal.hpp#L174-L182)),
so `disable`, `addreserved`, `sdr_brightness` and `Mode` are unknown keys.
The **Default** column is the effective value when the key is omitted from a
fresh rule: it comes from the member initialisers of `Config::CMonitorRule`
([MonitorRule.hpp L41-69](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L41-L69)),
not from the factory values in `MONITOR_FIELDS`, which only seed throw-away
parser objects for keys that are present
([LuaBindingsConfigRules.cpp L1142-1156](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L1142-L1156)).
For a merged rule (§1.5) an omitted key keeps the earlier call's value.

| Key | Lua type | Accepted values | Default (omitted) | hyprlang equivalent | Notes |
|---|---|---|---|---|---|
| `output` | string (a number is converted) | connector name (`DP-1`), `desc:<prefix>`, `""` = fallback rule | required | first field of `monitor=`; `monitorv2 { output = ... }` | §9. Missing or other type: error, no rule ([L1113-1120](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L1113-L1120)) |
| `mode` | string (number accepted) | `""` or `pref*` = preferred; `highrr*`; `highres*`; `maxwidth*`; `"WxH"`; `"WxH@Hz"`; `"modeline ..."` | preferred, refresh target 60 Hz | 2nd field; `monitorv2` `mode` | Prefix matches are case-sensitive; bad value → preferred + error. §12.3 ([Parser.cpp L110-143](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L110-L143)) |
| `position` | string (number accepted) | `"XxY"` (integers, negatives allowed); `""`, `auto`, `auto-right`, `auto-left`, `auto-up`, `auto-down`, `auto-center-right`, `auto-center-left`, `auto-center-up`, `auto-center-down` | auto (treated as right) | 3rd field; `monitorv2` `position` | A table such as `{x=0,y=0}` is rejected. §12.2 ([Parser.cpp L145-191](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L145-L191)) |
| `scale` | string or number | `""` or `auto*` = auto; a number ≥ 0.25 | auto (`-1`, PPI based) | 4th field; `monitorv2` `scale` | Snapped at apply time (§4). < 0.25 → 1; non-numeric → unchanged ([Parser.cpp L193-211](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L193-L211)) |
| `transform` | integer or boolean | 0-7 | 0 | extra `transform,N`; short form `SEL,transform,N`; `monitorv2` `transform` | Cast straight to `wl_output_transform`. §12.1 ([L103-107](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L103-L107)) |
| `disabled` | boolean, or number 0/1 | `true`, `false` | `false` | `SEL,disable` / `SEL,disabled`; `monitorv2` `disabled` | `false` re-enables since 0.56.0 ([L98-102](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L98-L102)) |
| `mirror` | string (number accepted) | a monitor query string (§9.4); `""` = no mirror | no mirror | extra `mirror,X`; `monitorv2` `mirror` | Stored unvalidated ([Parser.cpp L275-290](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L275-L290)) |
| `bitdepth` | integer or boolean | any integer; only `10` has an effect | 8-bit | extra `bitdepth,10`; `monitorv2` `bitdepth` | 8, 12 or `true` all mean 8-bit; 10-bit falls back to 8-bit formats if unsupported ([Monitor.cpp L980-987](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L980-L987)) |
| `cm` | string (number accepted) | `auto`, `srgb`, `wide`, `edid`, `hdr`, `hdredid`, `dcip3`, `dp3`, `adobe` | `srgb` | extra `cm,X`; `monitorv2` `cm` | Unknown → error, value unchanged ([CMType.cpp L6-15](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/helpers/CMType.cpp#L6-L15), [Parser.cpp L234-242](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L234-L242)) |
| `sdr_eotf` | string (number accepted) | `default` or `"0"`; `auto`; `srgb` or `"3"`; `gamma22` or `"1"`; `gamma22force` or `"2"` | `default` | `monitorv2` `sdr_eotf` only (≥ 0.52, different digit meanings, §8.2) | Unknown strings silently become `default` ([TransferFunction.cpp L10-19](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/helpers/TransferFunction.cpp#L10-L19)) |
| `sdrbrightness` | number, boolean or numeric string | float, no range | 1.0 | extra `sdrbrightness,F`; `monitorv2` | ([L125-129](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L125-L129)) |
| `sdrsaturation` | number, boolean or numeric string | float, no range | 1.0 | extra `sdrsaturation,F`; `monitorv2` | ([L130-134](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L130-L134)) |
| `vrr` | integer or boolean | -1 (follow `misc:vrr`), 0 off, 1 on, 2 fullscreen only, 3 fullscreen with game/video content type | unset = follow `misc:vrr` (default 0) | extra `vrr,N`; `monitorv2` `vrr` | Strings such as `"fullscreen"` are rejected; `misc:vrr` itself accepts `off`/`on`/`fullscreen`/`fullscreen_game` ([L135-140](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L135-L140), [ConfigValues.cpp L483-484](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/values/ConfigValues.cpp#L483-L484)) |
| `icc` | string (number accepted) | a path; `""` is rejected | none | extra `icc,PATH` (≥ 0.55); `monitorv2` `icc` (≥ 0.55) | Path not checked at parse time ([Parser.cpp L275-290](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L275-L290)) |
| `reserved`, `reserved_area` | integer (all sides) or table `{top, right, bottom, left}` | integers; missing side = 0 | 0 | `SEL,addreserved,T,B,L,R`; `monitorv2` `addreserved = "T,B,L,R"` | Aliases with identical code; emit only one (key order is unspecified) ([L88-97](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L88-L97)) |
| `supports_wide_color` | integer or boolean | -1 force off, 0 auto, 1 force on | 0 | `monitorv2` only | ([MonitorRule.hpp L58](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L58)) |
| `supports_hdr` | integer or boolean | -1, 0, 1 as above | 0 | `monitorv2` only | ([MonitorRule.hpp L59](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L59)) |
| `sdr_min_luminance` | number | float, no range | 0.2 | `monitorv2` only | ([MonitorRule.hpp L60](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L60)) |
| `sdr_max_luminance` | integer or boolean | any integer | 80 | `monitorv2` only | ([MonitorRule.hpp L61](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L61)) |
| `min_luminance` | number | float; ≥ 0 overrides EDID | -1 (EDID) | `monitorv2` only | ([MonitorRule.hpp L64](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L64)) |
| `max_luminance` | integer or boolean | any integer; ≥ 0 overrides EDID | -1 (EDID) | `monitorv2` only | ([MonitorRule.hpp L65](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L65)) |
| `max_avg_luminance` | integer or boolean | any integer; ≥ 0 overrides EDID | -1 (EDID) | `monitorv2` only | ([MonitorRule.hpp L66](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L66)) |

Key-to-member mapping: `output`→`m_name`; `mode`→`m_resolution`,
`m_refreshRate`, `m_drmMode`; `position`→`m_offset`, `m_autoDir`;
`scale`→`m_scale`; `reserved`/`reserved_area`→`m_reservedArea`;
`disabled`→`m_disabled`; `transform`→`m_transform`; `mirror`→`m_mirrorOf`;
`bitdepth`→`m_enable10bit` (`== 10`); `cm`→`m_cmType`; `sdr_eotf`→`m_sdrEotf`;
`vrr`→`m_vrr` (`nullopt` if < 0); `icc`→`m_iccFile`; the rest map to the
member of the same name
([LuaBindingsConfigRules.cpp L81-178](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L81-L178),
[Parser.hpp L6-31](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.hpp#L6-L31)).

The generated stub types match: `output string`, `position? string`,
`reserved? integer|HL.CssGap`, `scale? string|number`,
`transform? integer|boolean`, `vrr? integer|boolean`, with `output` the only
required field
([generateLuaStubs.py L368-370, L755-760](https://github.com/hyprwm/Hyprland/blob/v0.56.2/meta/generateLuaStubs.py#L368-L370)).

### 1.3 Lua value parsing per type

| Value class | Keys | Accepts | Rejects (message) | Source |
|---|---|---|---|---|
| `CLuaConfigString` | `mode`, `position`, `scale`, `mirror`, `cm`, `sdr_eotf`, `icc` | anything `lua_isstring` accepts: strings and numbers (converted with `lua_tostring`) | booleans, tables: `string type requires a string` | [LuaConfigString.cpp L13-29](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/types/LuaConfigString.cpp#L13-L29) |
| `CLuaConfigInt` | `transform` (0..7), `vrr` (-1..3), `supports_wide_color`, `supports_hdr` (-1..1), `bitdepth`, `sdr_max_luminance`, `max_luminance`, `max_avg_luminance` (no range) | Lua integer subtype; boolean (`true`→1, `false`→0) | float subtype such as `1.0` and any string such as `"1"` (no string map for these keys): `integer type requires a bool or an integer`; out of range: `value N is more than the maximum of M` / `value N is less than the minimum of M` | [LuaConfigInt.cpp L11-64](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/types/LuaConfigInt.cpp#L11-L64), [LuaConfigInt.hpp L12](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/types/LuaConfigInt.hpp#L12) |
| `CLuaConfigFloat` | `sdrbrightness`, `sdrsaturation`, `sdr_min_luminance`, `min_luminance` | boolean, integer, anything `lua_isnumber` accepts (including `"1.2"`) | `float type requires a number` | [LuaConfigFloat.cpp L10-56](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/types/LuaConfigFloat.cpp#L10-L56) |
| `CLuaConfigBool` | `disabled` | `true`/`false`; a number (or numeric string) equal to 0 or 1 | other numbers: `boolean type requires a bool, or 0/1.`; other types: `boolean type requires a bool` | [LuaConfigBool.cpp L10-28](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/types/LuaConfigBool.cpp#L10-L28) |
| `CLuaConfigCssGap` | `reserved`, `reserved_area` | a number (any `lua_isnumber`, truncated with `sc<int64_t>`) for all sides; a table with optional `top`, `right`, `bottom`, `left` read with `lua_tointeger` | unknown table keys are ignored silently; a non-integral field such as `top = 10.5` becomes 0 | [LuaConfigCssGap.cpp L18-68](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/types/LuaConfigCssGap.cpp#L18-L68) |

The integer check is:

```cpp
// src/config/lua/types/LuaConfigInt.cpp (v0.56.2)
if (lua_isstring(s, -1) && !lua_isinteger(s, -1)) {
    if (!m_map.has_value())
        return {.errorCode = PARSE_ERROR_BAD_TYPE, .message = "integer type requires a bool or an integer"};
```

([LuaConfigInt.cpp L28-30](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/types/LuaConfigInt.cpp#L28-L30)).
The live config writes `scale = 1` (a number) and it applies without error
(*live*, `~/.config/caelestia/hypr-user.lua` L8-10, `configerrors -j` = `[""]`).
How `lua_tostring` formats a non-terminating float such as `1.0666666666667`
before `stof` is not verified (§13); write non-integer scales as strings.

### 1.4 Errors and diagnostics

`hl.monitor` never calls `lua_error`; the rest of the configuration keeps
executing. Messages
([LuaBindingsConfigRules.cpp L1106-1160](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L1106-L1160)):

| Situation | Recorded error | Rule added? |
|---|---|---|
| Argument is not a table | `hl.monitor: argument must be a table` (no source prefix) | no |
| `output` missing, boolean, table, ... | `<src>:<line>: hl.monitor: 'output' field is required and must be a string` | no |
| Key that is not a string (array part) | none, skipped silently | yes |
| Unknown string key | `<src>:<line>: hl.monitor: unknown field '<key>'` | yes |
| Type or range error | `<src>:<line>: hl.monitor: field '<key>': <message>` (§1.3) | yes, field unchanged |
| Value the parser rejects | `<src>:<line>: hl.monitor: error applying field '<key>'`; the parser's own text (`invalid resolution `, ...) is never surfaced | yes, with fallback |

- `<src>` is the chunk source with a leading `@` removed and `<line>` the
  current line, taken at stack level 1
  ([LuaBindingsInternal.cpp L409-422](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsInternal.cpp#L409-L422)).
  It is absolute for files found through the XDG path, Caelestia's
  `package.path` entries or a canonical `-c` path; it is relative for modules
  found through Lua's default `./?.lua` or a relative `HYPRLAND_CONFIG`. If
  `hl.monitor` is called through a helper function, the helper's file and line
  are reported.
- Fallbacks written into the committed rule
  ([Parser.cpp L110-211](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L110-L211)):
  bad mode → resolution `(0,0)` = preferred; bad `XxY` position → auto;
  bad `auto-*` direction → offset auto, `m_autoDir` unchanged (fresh rule:
  `DIR_AUTO_NONE`, treated as right); numeric scale < 0.25 → 1; non-numeric
  scale → unchanged (fresh rule: `-1` = auto, which on the maintainer's eDP-1
  means 1.5, §4); unknown `cm` or empty `icc` → unchanged.
- Keys are visited in `lua_next` order, which Lua leaves unspecified: the
  order of errors and the winner between `reserved` and `reserved_area` in one
  call are not deterministic
  ([L1128-1158](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L1128-L1158)).
- `addError` appends to `m_errors` (read by `hyprctl configerrors` and the
  red overlay) only while the config is parsed or `eval`uated; at any other
  time (event handler, timer, keybind) it becomes a 5 s notification
  `Runtime error in lua:\n...` and never reaches `configerrors`
  ([ConfigManager.cpp L857-870](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L857-L870)).
- The overlay stops after more than 15 newlines including the
  `Your config has errors:` header and is hidden by `debug:suppress_errors`
  ([ConfigManager.cpp L763-796](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L763-L796)).

### 1.5 Accumulation: calls with the same output string merge

```cpp
// src/config/shared/monitor/MonitorRuleManager.cpp L37-42 (v0.56.2)
void CMonitorRuleManager::add(CMonitorRule&& x) {
    std::erase_if(m_rules, [&x](const auto& e) { return e.m_name == x.m_name; });
    m_rules.emplace_back(std::move(x));

    scheduleReload();
}
```

- `hlMonitor` copies an existing rule whose `m_name` equals the new `output`
  string exactly, applies only the keys present, and `add()` erases the old
  entry and appends the merged rule at the end
  ([LuaBindingsConfigRules.cpp L1122-1126](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L1122-L1126),
  [MonitorRuleManager.cpp L37-42](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRuleManager.cpp#L37-L42)).
  Omitted fields keep earlier values; even a call with no valid field re-adds
  the copy and moves it to the end. This applies during config load and to
  `eval`. v0.55.0 already merges.
- Consequence: a managed block placed after a user's
  `hl.monitor({output = "DP-1", vrr = 2, transform = 1})` inherits `vrr` and
  `transform` for every key it omits. A block that owns an output must write
  every field it owns, or read the earlier calls.
- Stale modeline (inference, **needs runtime confirmation**): the `WxH` branch
  of `parseMode` never clears `m_drmMode`. After a modeline call, a later
  same-string call with `"2560x1440@144"` keeps `DRM_MODE_TYPE_USERDEF`, and
  `applyMonitorRule` tries the old modeline first. A failed modeline with at
  least 10 tokens but a non-numeric field has already set
  `m_drmMode.type = DRM_MODE_TYPE_USERDEF` with partial data
  ([Parser.cpp L48-58](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L48-L58),
  [Monitor.cpp L843-848](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L843-L848)).
  Emit one call per output string.
- Legacy `monitor=` does not merge (§8.1).

### 1.6 Version history of hl.monitor

| Version | Change | Source |
|---|---|---|
| 0.55.0 | `hl.monitor` with merge-on-same-output introduced (5ba33f84, #13817, 2026-04-26) | [commit](https://github.com/hyprwm/Hyprland/commit/5ba33f846129f3e6c0505b19f94634b24505e828) |
| 0.55.x | Omitted `mode`/`position` gave **1280x720 at 0x0**; empty strings were not preferred/auto | [commit 049595e1](https://github.com/hyprwm/Hyprland/commit/049595e196db4a4ab162ce58aadd016e929327c8) |
| 0.56.0 | `disabled = false` re-enables (b76a9e07, #14447; before, `false` was a no-op) | [commit](https://github.com/hyprwm/Hyprland/commit/b76a9e07) |
| 0.56.0 | `vrr` range -1..3, -1 inherits `misc:vrr` (faecc595, #14746) | [commit](https://github.com/hyprwm/Hyprland/commit/faecc595) |
| 0.56.0 | Defaults changed to preferred / auto, `""` means preferred / auto (049595e1, #15193) | [commit](https://github.com/hyprwm/Hyprland/commit/049595e196db4a4ab162ce58aadd016e929327c8) |
| main | `hlMonitor`, `MONITOR_FIELDS`, parser semantics and selector matching unchanged | [main LuaBindingsConfigRules.cpp](https://github.com/hyprwm/Hyprland/blob/4bb6844b0351e4fbf2e3d4e46ae71b551a0e0a42/src/config/lua/bindings/LuaBindingsConfigRules.cpp) |

To support 0.55.x, always write `mode` and `position` explicitly.

### 1.7 Reference configurations

- Upstream example: `hl.monitor({ output = "", mode = "preferred", position = "auto", scale = "auto" })`
  ([example/hyprland.lua L18-23](https://github.com/hyprwm/Hyprland/blob/v0.56.2/example/hyprland.lua#L18-L23)).
- Caelestia: the same catch-all with `scale = 1`
  ([hypr/hyprland.lua L53-59](https://github.com/caelestia-dots/caelestia/blob/d459e9182ae55ba7eb29fa90781297224911d6bd/hypr/hyprland.lua#L53-L59)).
- Maintainer (*live*, `hypr-user.lua` L8-10), all applied without errors:

```lua
hl.monitor({ output = "HDMI-A-1", mode = "2560x1440@144",    position = "0x0",       scale = 1, transform = 1 })
hl.monitor({ output = "eDP-1",    mode = "1920x1080@144",    position = "1440x1335", scale = 1 })
hl.monitor({ output = "DP-1",     mode = "2560x1440@179.95", position = "3360x975",  scale = 1, vrr = 2 })
```

## 2. Live changes: keyword, eval, reload

### 2.1 Mechanisms

| Mechanism | Config type | Since | Reply | Lifetime | Source |
|---|---|---|---|---|---|
| `hyprctl eval '<lua>'` | Lua only (else `eval is only supported with the lua config manager`) | 0.55.0 | `ok`, or `error: ...` lines | until the next reload | [HyprCtl.cpp L1110-1124](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L1110-L1124) |
| `hyprctl repl '<lua>'` | Lua only | 0.56.0 | returned or printed values, instead of the error list | until the next reload | [HyprCtl.cpp L2043-2045](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L2043-L2045) |
| `hyprctl keyword monitor <value>` | legacy only; under Lua: `keyword can't work with non-legacy parsers. Use eval.` (guard since 0.55.0) | before 0.55 (not dated here) | handler result | until the next reload | [HyprCtl.cpp L1160-1201](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L1160-L1201) |
| `hyprctl reload` | both | — | always `ok` | re-reads files | [HyprCtl.cpp L1260-1279](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L1260-L1279) |
| `hyprctl reload full-reset` | both | 0.55.3 (84ccaf3d) / 0.56.0 (e43b163f) | `ok` | re-resolves the config path and manager | same |
| write a watched file | both | — | — | autoreload (§10.2) | [ConfigWatcher.cpp L32-111](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/inotify/ConfigWatcher.cpp#L32-L111) |

`eval`, `status` and the `keyword` guard exist from v0.55.0 and are absent in
v0.54.3; `repl` first appears in v0.56.0 (git history of
[HyprCtl.cpp](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L2003-L2046)).
**main:** no `keyword` command (legacy removed by
[a9902ea6](https://github.com/hyprwm/Hyprland/commit/a9902ea65f461af868497c4e74fed798ade0b1b0)).

### 2.2 eval

```cpp
// src/config/lua/ConfigManager.cpp (v0.56.2), eval()
if (luaL_loadstring(m_lua, code.starts_with("return") ? code.c_str() : std::format("return {};", code).c_str()) != LUA_OK) {
...
if (guardedPCall(0, LUA_MULTRET, 0, LUA_TIMEOUT_EVAL_MS, "hyprctl eval") != LUA_OK) {
...
    return std::format("error: {}", err);
```

([ConfigManager.cpp L880-953](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L880-L953))

- The code is the request text after the first space. It is first compiled
  as `return <code>;` (unless it already starts with `return`), falling back
  to the raw code, and runs in the **same persistent `lua_State`** as the
  config, with a 250 ms watchdog (`LUA_TIMEOUT_EVAL_MS`,
  [ConfigManager.hpp L113](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.hpp#L113)).
  No permission type or option gates it
  ([DynamicPermissionManager.hpp L17-22](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/managers/permissions/DynamicPermissionManager.hpp#L17-L22)).
- Replies: `ok`; `error: <msg>` for a syntax or runtime error (then `m_errors`
  is ignored); otherwise one `error: ...` line per `addError` plus
  `<level>: msg` for warnings. **main:** syntax errors read
  `error: <code> <msg>`.
- Every `eval` starts with `m_errors.clear()`, so the errors of the last
  config load disappear from `configerrors`
  ([ConfigManager.cpp L884-888](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L884-L888)).
  In Lua mode `hyprctl dispatch` is implemented through `eval`, so any client
  running a dispatch empties `configerrors` too
  ([HyprCtl.cpp L1126-1139](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L1126-L1139)).
  `configerrors` is reliable only right after a reload.
- `hl.monitor` through `eval` merges into the rule with the same output
  string and moves it to the end (§1.5). An `error:` reply does **not** mean
  nothing was applied: the rule is still added with the valid fields
  ([LuaBindingsConfigRules.cpp L1103-1165](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L1103-L1165)).
  An `eval` cannot unset a merged field such as `mirror`; only a reload clears it.
- Application is asynchronous. `add()` only sets `m_reloadScheduled`;
  `REFRESH_MONITOR_STATES` calls `scheduleReload()` and `ensureVRR()` (with the
  *old* active rule) and recalculates layouts; `ensureMonitorStatus()` runs from
  the `render.preChecks` listener on the next rendered frame, after `eval` has
  already replied `ok`. No frame is scheduled, so on an idle screen
  application can be delayed (**needs runtime confirmation**)
  ([MonitorRuleManager.cpp L24-31](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRuleManager.cpp#L24-L31),
  [PropRefresher.cpp L116-128](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/supplementary/propRefresher/PropRefresher.cpp#L116-L128),
  [Renderer.cpp L2049-2052](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/render/Renderer.cpp#L2049-L2052)).
  After a preview, poll `monitors all -j` with a timeout.
- `ensureMonitorStatus` compares the new rule with the active one: changes to
  position, transform, mirror, reserved area, cm or auto direction are a
  *soft* mismatch (`applyMonitorRuleSoft`, no modeset, no scale check);
  changes to enabled, resolution, refresh, scale, bitdepth or modeline need a
  full modeset (`applyMonitorRule`)
  ([MonitorRule.cpp L7-34](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.cpp#L7-L34),
  [MonitorRuleManager.cpp L134-205](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRuleManager.cpp#L134-L205)).
  Enable state changes call `onDisconnect()` / `onConnect(true)`
  ([L186-187](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRuleManager.cpp#L186-L187)).
- **VRR-only changes** (inference, **needs runtime confirmation**):
  `CMonitorRule::compare` ignores `m_vrr`, so a rule that differs only in
  `vrr` is a full match, the monitor is skipped (`L159`) without updating
  `m_activeMonitorRule`, and `ensureVRR` keeps reading the stale value
  (`L214`). This affects `eval` and reload alike
  ([MonitorRule.cpp L7-34](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.cpp#L7-L34),
  [MonitorRuleManager.cpp L159-214](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRuleManager.cpp#L159-L214)).
- Never send `eval` inside `[[BATCH]]` (§3.3).

### 2.3 reload (Lua manager)

```cpp
// src/config/lua/ConfigManager.cpp L682-701 (v0.56.2)
        if (luaL_loadfile(m_lua, m_mainConfigPath.c_str()) != LUA_OK) {
            m_errors.clear();
            addError(lua_tostring(m_lua, -1));
            lua_pop(m_lua, 1);
            return false;
        }

        return true;
    };

    if (!phase1Load()) {
        m_lastConfigVerificationWasSuccessful = false;
        postConfigReload();
        return;
    }

    // phase 2: syntax is valid, reset and load.
    Config::animationTree()->reset();
    Config::workspaceRuleMgr()->clear();
    Config::monitorRuleMgr()->clear();
```

Sequence ([ConfigManager.cpp L634-760](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L634-L760),
[L809-855](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L809-L855)):

1. `m_mainConfigPath` is taken from the **cached** discovery result;
   `m_configPaths` is reset to `[main]`.
2. Phase 1: `package.loaded[k] = nil` for every non-stdlib key of the current
   state, then `luaL_loadfile` of the **main file only**. On failure
   `m_errors` becomes that single error, all rules stay, `postConfigReload()`
   still runs and posts `configreloaded`.
3. Phase 2: clears animations, workspace/monitor/window/layer rules,
   gestures, timers, layout providers, errors, device configs, plugins, event
   handlers and binds ([L698-720](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L698-L720)).
4. `reinitLuaState()`: `lua_close` + `luaL_newstate`; phase 1 again on the
   new state (if it now fails, rules stay cleared); every config value is
   reset to its default; the main chunk runs with a traceback handler,
   `nresults = 0` and a 1500 ms watchdog covering all modules
   ([L722-751](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L722-L751),
   [ConfigManager.hpp L107-108](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.hpp#L107-L108)).
5. `postConfigReload()`: unless first launch, `scheduleReload()`,
   `ensureMonitorStatus()` and `ensureVRR()` **synchronously**, then
   `REFRESH_ALL` and `configreloaded`
   ([L812-818](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L812-L818),
   [L853-854](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L853-L854)).

Consequences: rules and globals created by `eval` are lost; to revert a
preview, send `reload`, after which `configerrors` reflects the files on disk.
Syntax errors in `require`d files are **not** caught by phase 1 (§5.8).
Whether `monitors all -j` already shows the new DRM state when the `reload`
reply arrives needs runtime confirmation.

Legacy reload has no syntax pre-check: it clears monitor rules, then parses
([legacy ConfigManager.cpp L726-751](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L726-L751)),
and posts `configreloaded` unconditionally
([L1074](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1074)).

### 2.4 keyword (legacy manager)

- `keyword monitor <value>` sets `isDynamicKeyword`, calls
  `parseKeyword(COMMAND, VALUE)` = `parseDynamic`, and any `COMMAND` containing
  `monitor` schedules `monitorRuleMgr()->scheduleReload()` via `doLater`
  ([HyprCtl.cpp L1188-1201](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L1188-L1201),
  [legacy ConfigManager.cpp L1089-1090](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1089-L1090)).
- `parseDynamic(cmd, value)` is `parseLine(cmd + "=" + value, true)`: a `#` in
  the value starts a comment and `$vars` expand
  ([hyprlang config.cpp L1064-1068](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L1064-L1068)).
- It is the same `handleMonitor` as the file (§8.1). Rules set this way are
  appended after the file's rules and dropped on reload. Not run live
  (**needs runtime confirmation**). `keyword monitorv2[...]:...` creates no
  rule before a reload, and the reload re-parses the file and drops it
  (**needs runtime confirmation**); use the `monitor=` form for previews.

### 2.5 hyprctl exit codes

`hyprctl` exits with 7 only if the reply starts with `error:`
([hyprctl/src/main.cpp L209-280](https://github.com/hyprwm/Hyprland/blob/v0.56.2/hyprctl/src/main.cpp#L209-L280)).
`keyword can't work ...`, `eval is only supported ...`, `unknown request`,
`too many args`, `Invalid dispatcher` and `Err: <what>` all exit 0. Parse the
reply text.

## 3. Reading state via IPC

### 3.1 Socket paths

| Item | Value | Source |
|---|---|---|
| Runtime root (compositor) | `$XDG_RUNTIME_DIR/hypr`; throws if the result starts with `/hypr` (empty variable); an unset variable is undefined behaviour (`std::string` from `nullptr`). No `/tmp/hypr` fallback | [Compositor.cpp L192-227](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/Compositor.cpp#L192-L227) |
| Instance dir | `<root>/<HYPRLAND_INSTANCE_SIGNATURE>`, `mkdir` mode `S_IRWXU` (subject to umask; root only created if missing) | same |
| Request socket | `<instance>/.socket.sock`, `AF_UNIX SOCK_STREAM`, backlog 10 | [HyprCtl.cpp L2321-2346](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L2321-L2346) |
| Event socket | `<instance>/.socket2.sock` (non-blocking; refused if longer than `sun_path - 1`) | [EventManager.cpp L14-21](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/managers/EventManager.cpp#L14-L21) |
| `hyprctl` client root | `$XDG_RUNTIME_DIR/hypr`; `/run/user/<uid>/hypr` only when `XDG_RUNTIME_DIR` is unset | [hyprctl/src/main.cpp L81-90](https://github.com/hyprwm/Hyprland/blob/v0.56.2/hyprctl/src/main.cpp#L81-L90) |
| Signature | `"{GIT_COMMIT_HASH}_{unix time}_{random int}"`, exported with `setenv`; *live*: `efb50993780079460b0cbed1363e2166a2de1d9f_1790602009_967952317` | [Compositor.cpp L206-209](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/Compositor.cpp#L206-L209) |
| Lock file | `<instance>/hyprland.lock` = `"<pid>\n<wayland socket>\n"`; `hyprctl instances` needs exactly 2 lines, takes the time between the first and last `_`, drops dead PIDs, sorts by time; *live*: pid 3436, `wayland-1` | [Compositor.cpp L765-773](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/Compositor.cpp#L765-L773), [hyprctl/src/main.cpp L101-138](https://github.com/hyprwm/Hyprland/blob/v0.56.2/hyprctl/src/main.cpp#L101-L138) |

**main:** same paths ([main src/ipc/s1/Unix.cpp L242](https://github.com/hyprwm/Hyprland/blob/4bb6844b0351e4fbf2e3d4e46ae71b551a0e0a42/src/ipc/s1/Unix.cpp#L242)).

### 3.2 Request wire format

- A request is `<flags>/<command args>`. Flag parsing is skipped for requests
  starting with `[[BATCH]]` or containing no `/`; otherwise characters up to
  the first `/` are flags, unless a space comes first (then nothing is
  stripped). `j` = JSON, `r` = refresh everything afterwards, `a` = all,
  `c` = include config in `systeminfo`; other characters are consumed and
  ignored ([HyprCtl.cpp L2062-2097](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L2062-L2097)).
  Always send a prefix (`j/monitors all`, `/eval ...`) so a `/` inside Lua is
  never parsed as a flag separator.
- The `hyprctl` client sends `fullArgs + "/" + argv joined by spaces`
  ([main.cpp L480](https://github.com/hyprwm/Hyprland/blob/v0.56.2/hyprctl/src/main.cpp#L480)),
  except `--batch`, which sends `"[[BATCH]]" + commands`; with `-j` it rewrites
  every `;\s*` to `;j/` and prefixes `j/`, corrupting multi-statement Lua
  ([main.cpp L339-349](https://github.com/hyprwm/Hyprland/blob/v0.56.2/hyprctl/src/main.cpp#L339-L349)).
- Matching: exact commands first, then prefix commands in registration order;
  no match (or an empty handler result, e.g. plain `monitors` with no
  monitors) returns `unknown request`. Exact: `workspaces`,
  `workspacerules`, `activeworkspace`, `clients`, `kill`, `activewindow`,
  `layers`, `version`, `devices`, `splash`, `cursorpos`, `binds`,
  `globalshortcuts`, `systeminfo`, `animations`, `rollinglog`,
  `configerrors`, `locked`, `descriptions`, `submap`, `status`. Prefix:
  `reloadshaders`, `monitors`, `reload`, `plugin`, `notify`,
  `dismissnotify`, `getprop`, `seterror`, `switchxkblayout`, `output`,
  `dispatch`, `keyword`, `setcursor`, `getoption`, `decorations`,
  `[[BATCH]]`, `eval`, `repl`
  ([HyprCtl.cpp L2003-2046](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L2003-L2046),
  [L2101-2124](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L2101-L2124)).

### 3.3 Framing, batch and client rules

- The server `accept4`s a **blocking** socket (`SOCK_CLOEXEC` only), polls
  5000 ms for the first data (silent close on timeout), then reads 1023-byte
  chunks into a zero-filled 1024-byte buffer (each chunk truncated at its own
  first NUL) until a read returns < 1 or < 1023 bytes. An uncaught exception
  replies `Err: <what>`. The reply is written in full with no length prefix or
  terminator by a blocking `write` with no timeout, then the socket is closed.
  Exceptions: a pending plugin promise, and any request that contains
  `rollinglog` and `f` (kept open for log following)
  ([HyprCtl.cpp L2228-2315](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L2228-L2315),
  [L2176-2208](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L2176-L2208)).
- A request whose length is a multiple of 1023, or one split across several
  writes, makes the server block in `read()` or truncate (inference, **needs
  runtime confirmation**). A client that does not read a large reply (e.g.
  `monitors all -j`) stalls the compositor.
- **Client rule for hyprtilt:** one `write()` of the whole request,
  `shutdown(SHUT_WR)`, then read to EOF immediately. v0.56.2 has no size limit.
  **main:** non-blocking reads, 1 MiB `REQUEST_LIMIT`, 5 s per-peer timer
  ([main s1/Unix.cpp L43, L81-82](https://github.com/hyprwm/Hyprland/blob/4bb6844b0351e4fbf2e3d4e46ae71b551a0e0a42/src/ipc/s1/Unix.cpp#L43)).
- `[[BATCH]]`: the 9-char token is stripped, the rest split on `;` at `[ ]`
  depth 0 (every `[`/`]` counts, including Lua indexing), each part trimmed and
  run through `getReply` with its own flags, empty parts skipped, replies
  joined with `\n\n\n` (trailing delimiter stripped); no backslash escaping
  ([HyprCtl.cpp L1309-1333](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L1309-L1333)).
  **main:** backslash escaping, no bracket depth
  ([main s1/S1.cpp](https://github.com/hyprwm/Hyprland/blob/4bb6844b0351e4fbf2e3d4e46ae71b551a0e0a42/src/ipc/s1/S1.cpp#L144-L145)).
  Send `eval` standalone.

### 3.4 Replies

**`monitors -j` / `monitors all -j`.** `monitors` lists `m_monitors`
(enabled, mirrors removed on every `layoutChanged`); `monitors all` lists
`m_realMonitors` (every real output, disabled and mirrors included). Use
`all`. Objects are omitted for monitors with no `m_output` or `m_id == -1`;
IDs are reused per output name when free
([HyprCtl.cpp L313-344](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L313-L344),
[MonitorState.cpp L24-75](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/state/MonitorState.cpp#L24-L75)).
The 37 fields, in order ([HyprCtl.cpp L223-288](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L223-L288)):

| Field | JSON type | Meaning |
|---|---|---|
| `id` | int | monitor ID |
| `name` | string | connector name (`DP-1`) |
| `description` | string | `m_shortDescription` = trim(`make model serial`), commas removed (§9) |
| `make`, `model`, `serial` | string | raw output strings |
| `width`, `height` | int | current mode's pixel size, **not** rotated (*live*: HDMI-A-1 at transform 1 reports 2560 x 1440) |
| `physicalWidth`, `physicalHeight` | int | millimetres |
| `refreshRate` | number, `{:.5f}` | Hz (*live*: `179.95200`) |
| `x`, `y` | int | layout position in logical pixels, truncated from double |
| `activeWorkspace`, `specialWorkspace` | object `{id, name}` | **main:** adds `address`, `type`; `id` only for numbered workspaces |
| `reserved` | array of 4 int | **left, top, right, bottom** |
| `scale` | number, `{}` format | effective (corrected) scale; 1.0 prints as `1` |
| `transform` | int | 0-7 |
| `focused`, `dpmsStatus` | bool | |
| `vrr` | bool | current adaptive-sync state, **not** the rule's 0-3 mode |
| `solitary`, `directScanoutTo` | string | hex pointer without `0x` |
| `solitaryBlockedBy`, `tearingBlockedBy`, `directScanoutBlockedBy` | null or array of string | |
| `activelyTearing` | bool | |
| `disabled` | bool | `!m_enabled` |
| `currentFormat` | string | `XRGB2101010`, `XBGR2101010`, `XRGB8888`, `XBGR8888` or `Invalid` ([L111-121](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L111-L121)) |
| `mirrorOf` | **string** | mirrored monitor's numeric ID (`"1"`) or `"none"`; not a name |
| `availableModes` | array of string | `"{w}x{h}@{mHz/1000:.2f}Hz"`, duplicates possible ([L123-136](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L123-L136)) |
| `colorManagementPreset` | string | resolved `m_cmType` (auto → wide or srgb; edid/hdr fall back to srgb without support), `""` if unknown; not the configured `cm` ([Monitor.cpp L662-668](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L662-L668)) |
| `sdrBrightness`, `sdrSaturation`, `sdrMinLuminance`, `sdrMaxLuminance` | number | |
| `hardwareCursorsInUse` | bool | |

Practical consequences:
- There is no logical-size field; compute it (§4).
- Map `mirrorOf` to a name through the `id` of another entry before writing
  `mirror = "<name>"`.
- The configured `vrr` and `cm` cannot be read back; take them from the file
  ([HyprCtl.cpp L283](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L283)).
- *Live*: HDMI-A-1 lists 35 modes with `1280x720@60.00Hz` twice; DP-1 lists 36
  with two duplicates. Deduplicate in the UI.
- **main:** adds `hardwareDetails {backend, hdr, chroma, bt2020, vrrCapable}`
  ([main s1/Commands.cpp L268](https://github.com/hyprwm/Hyprland/blob/4bb6844b0351e4fbf2e3d4e46ae71b551a0e0a42/src/ipc/s1/Commands.cpp#L268)).
  Parse leniently.

**`configerrors -j`** splits `getErrors()` on `\n` (each line trimmed) into
an array; with no errors it returns `[""]`, not `[]` (*live*:
`[\n\t""\n]\n`); the plain form prints a newline. Multi-line errors
(tracebacks) become several entries
([HyprCtl.cpp L725-746](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L725-L746)).
Drop empty strings before deciding.

**`status`** (exact command, 0.55+): with `j`,
`{"configProvider": "lua"|"hyprlang"|"error", "backend": "drm"|"wayland"|"error"}`;
plain: `\nconfigProvider: <x>\nbackend: <y>\n`. `systeminfo` ignores `j` and
ends with `\n\nState:\n` + the plain status (*live*:
`configProvider: lua`, `backend: drm`), but it runs `lspci` synchronously on
the compositor thread; use `j/status`
([SystemInfo.cpp L29-64](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/helpers/SystemInfo.cpp#L29-L64),
[L199](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/helpers/SystemInfo.cpp#L199),
[L252-253](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/helpers/SystemInfo.cpp#L252-L253),
[config/ConfigManager.cpp L77-83](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/ConfigManager.cpp#L77-L83)).
`j/status` was not run live (not on the allowed list); its JSON is from source.

**`getoption <name> -j`** (*live*):
`{"option": "misc:disable_autoreload", "bool": false, "set": false }`.

### 3.5 Instances

`hyprctl instances -j` returns `instance`, `time`, `pid`, `wl_socket` per
running compositor (*live*: `"pid": 3436, "wl_socket": "wayland-1"`),
parsed from the lock files (§3.1).

### 3.6 Events (socket2)

Each line is exactly `<event>>><data>\n`; data is cut to its first 1024
**bytes** (may split UTF-8) and `\n` inside data becomes a space. Hyprland
never reads from socket2 clients. Each event is first `write()`n to the
non-blocking client; only events that hit `EAGAIN` are queued, and the client
is removed when 64 are already queued at a new post, or on a write error
([EventManager.cpp L126-192](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/managers/EventManager.cpp#L126-L192)).
Drain the socket continuously on its own thread.

| Event | Data | When | Source |
|---|---|---|---|
| `monitoradded` | `NAME` | end of `CMonitor::onConnect` only; the early returns (already enabled, rule disabled, non-desktop) skip it | [Monitor.cpp L260-401](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L260-L401) |
| `monitoraddedv2` | `ID,NAME,DESC` (`DESC` = short description) | same | [Monitor.cpp L387-388](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L387-L388) |
| `monitorremoved` | `NAME` | scope guard in `onDisconnect`, every path except shutdown | [Monitor.cpp L394-401](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L394-L401) |
| `monitorremovedv2` | `ID,NAME,DESC` | same | same |
| `configreloaded` | empty | end of `postConfigReload`, on every path including a failed phase-1 check | [ConfigManager.cpp L853-854](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L853-L854) |
| `focusedmon` | `MONNAME,WORKSPACENAME` | focus change | [FocusState.cpp L287-288](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/desktop/state/FocusState.cpp#L287-L288) |
| `focusedmonv2` | `MONNAME,WORKSPACEID` | focus change | same |
| `moveworkspace(v2)`, `activespecial(v2)` | not verified here | workspace events | [wiki ipc](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/ipc/_index.md) |

- Rule-driven disable/enable goes through `onDisconnect()` / `onConnect(true)`,
  so it also emits these events
  ([MonitorRuleManager.cpp L182-188](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRuleManager.cpp#L182-L188)).
  A monitor hot-plugged while its rule has `disabled = true` emits no
  `monitoradded`, yet its unplug emits `monitorremoved`.
- There is **no** event for a mode, position, scale, transform or VRR change
  of a connected monitor; the internal `monitor.layoutChanged` bus event is
  not exported. Refresh `monitors all -j` on `monitoradded*`,
  `monitorremoved*` and `configreloaded`, and poll after your own `eval`.
- *Live* format check: `windowtitle>>562a1d3b88c0\n`.
- **main:** same format, 64-event queue, names and payloads
  ([main s2/S2.cpp L20](https://github.com/hyprwm/Hyprland/blob/4bb6844b0351e4fbf2e3d4e46ae71b551a0e0a42/src/ipc/s2/S2.cpp#L20)).

## 4. Logical size rounding and scale validation

### 4.1 Parsing

```cpp
// src/config/shared/monitor/Parser.cpp L193-211 (v0.56.2)
bool CMonitorRuleParser::parseScale(const std::string& value) {
    if (value.empty() || value.starts_with("auto"))
        m_rule.m_scale = -1;
    else {
        if (!isNumber(value, true)) {
            m_error += "invalid scale ";
            return false;
        } else {
            m_rule.m_scale = stof(value);

            if (m_rule.m_scale < 0.25F) {
                m_error += "invalid scale ";
                m_rule.m_scale = 1;
                return false;
            }
        }
    }
    return true;
}
```

([Parser.cpp L193-211](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L193-L211)).
`isNumber(value, true)` accepts an optional leading `-`, digits, at most one
`.` that is not the first character, and must end with a digit: `".5"` and
`"1."` are rejected; `"-.5"` passes and then falls back to 1
([hyprutils String.cpp L50-84](https://github.com/hyprwm/hyprutils/blob/v0.14.1/src/string/String.cpp#L50-L84)).
The rule stores a `float`. The parser does no snapping.

### 4.2 Apply-time check and snapping

`autoScale` is `RULE->m_scale <= 0.1`
([Monitor.cpp L750](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L750));
`PDISABLESCALECHECKS` reads `debug:disable_scale_checks` (default `false`,
*live* `false`)
([Monitor.cpp L721](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L721)).
The check runs only in `applyMonitorRule` (full modeset):

```cpp
// src/output/Monitor.cpp L994-1051 (v0.56.2), CMonitor::applyMonitorRule
    m_setScale = m_scale;

    Vector2D logicalSize = m_pixelSize / m_scale;
    if (!*PDISABLESCALECHECKS && (logicalSize.x != std::round(logicalSize.x) || logicalSize.y != std::round(logicalSize.y))) {
        // invalid scale, will produce fractional pixels.
        // find the nearest valid.

        float    searchScale = std::round(m_scale * 120.0);
        bool     found       = false;

        double   scaleZero = searchScale / 120.0;

        Vector2D logicalZero = m_pixelSize / scaleZero;
        if (logicalZero == logicalZero.round())
            m_scale = scaleZero;
        else {
            for (size_t i = 1; i < 90; ++i) {
                double   scaleUp   = (searchScale + i) / 120.0;
                double   scaleDown = (searchScale - i) / 120.0;

                Vector2D logicalUp   = m_pixelSize / scaleUp;
                Vector2D logicalDown = m_pixelSize / scaleDown;

                if (logicalUp == logicalUp.round()) {
                    found       = true;
                    searchScale = scaleUp;
                    break;
                }
                if (logicalDown == logicalDown.round()) {
                    found       = true;
                    searchScale = scaleDown;
                    break;
                }
            }

            if (!found) {
                if (autoScale)
                    m_scale = std::round(scaleZero);
                else {
                    Log::logger->log(Log::ERR, "Invalid scale passed to monitor, {} failed to find a clean divisor", m_scale);
                    ErrorOverlay::overlay()->queueError("Invalid scale passed to monitor " + m_name + ", failed to find a clean divisor");
                    m_scale = getDefaultScale();
                }
            } else {
                if (!autoScale) {
                    Log::logger->log(Log::ERR, "Invalid scale passed to monitor, {} found suggestion {}", m_scale, searchScale);
                    static auto PDISABLENOTIFICATION = CConfigValue<Config::INTEGER>("misc:disable_scale_notification");
                    if (!*PDISABLENOTIFICATION) {
                        Notification::overlay()->addNotification(
                            I18n::i18nEngine()->localize(I18n::TXT_KEY_NOTIF_MONITOR_AUTO_SCALE,
                                                         {{"name", m_name}, {"scale", std::format("{:.2f}", m_scale)}, {"fixed_scale", std::format("{:.2f}", searchScale)}}),
                            CHyprColor(1.0, 0.0, 0.0, 1.0), 5000, ICON_WARNING);
                    }
                }
                m_scale = searchScale;
            }
        }
    }
```

([Monitor.cpp L994-1051](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L994-L1051)).
The en_US notification text is
`Invalid scale passed to monitor {name}: {scale}, using suggested scale: {fixed_scale}`
([i18n/Engine.cpp L249](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/i18n/Engine.cpp#L249)).
`m_setScale` keeps the requested (or PPI) value; `m_scale`, reported by
`hyprctl`, is the corrected one. The check uses the **untransformed**
`m_pixelSize`; both axes must be integral, so transform does not affect
validity ([Monitor.cpp L996-997](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L996-L997)).

Outcomes (verified against the code above):

| Case | Result | Log / UI |
|---|---|---|
| `pixelSize / scale` integral | scale kept | none |
| `scaleZero = round(scale*120)/120` valid | `m_scale = scaleZero` | none, even for an explicit scale |
| search hit, explicit scale (`i` = 1..89, up before down) | suggestion | ERR log + red 5000 ms notification, unless `misc:disable_scale_notification` |
| search hit, auto scale | suggestion | none |
| no hit, auto scale | `round(scaleZero)` (an integer) | none |
| no hit, explicit scale | `getDefaultScale()` | ERR log + ErrorOverlay `Invalid scale passed to monitor <name>, failed to find a clean divisor` |

### 4.3 Logical size

```cpp
// src/output/Monitor.cpp L1058-1060 (v0.56.2); same formula in applyMonitorRuleSoft L699-701
    Vector2D xfmd     = m_transform % 2 == 1 ? Vector2D{m_pixelSize.y, m_pixelSize.x} : m_pixelSize;
    m_size            = (xfmd / m_scale).round();
    m_transformedSize = xfmd;
```

([Monitor.cpp L1058-1060](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L1058-L1060),
[L699-701](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L699-L701)).
`Vector2D::round()` calls libm `round()` per component (half away from zero)
and `operator==` is exact (`a.x == x && a.y == y`) (*live*: `objdump` of
`/usr/lib/libhyprutils.so.0.14.2`, `/usr/include/hyprutils/math/Vector2D.hpp`
L43-45). `m_scale` is a `float` promoted to `double` for the division
([Monitor.hpp L67-78](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.hpp#L67-L78)).
Rust replica: store the scale as `f32`, divide in `f64`, compare exactly,
round with `f64::round` (same semantics).

### 4.4 Auto scale (PPI)

```cpp
// src/output/Monitor.cpp L1402-1418 (v0.56.2)
float CMonitor::getDefaultScale() {
    if (!m_output)
        return 1;

    static constexpr double MMPERINCH = 25.4;

    const auto              DIAGONALPX = sqrt(pow(m_pixelSize.x, 2) + pow(m_pixelSize.y, 2));
    const auto              DIAGONALIN = sqrt(pow(m_output->physicalSize.x / MMPERINCH, 2) + pow(m_output->physicalSize.y / MMPERINCH, 2));

    const auto              PPI = DIAGONALPX / DIAGONALIN;

    if (PPI > 200 /* High PPI, 2x*/)
        return 2;
    else if (PPI > 140 /* Medium PPI, 1.5x*/)
        return 1.5;
    return 1;
}
```

([Monitor.cpp L1402-1418](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L1402-L1418)).
An output reporting 0 x 0 mm gives PPI = +inf and scale 2. *Live*: eDP-1
(1920x1080, 340x190 mm) has PPI 143.66 → auto = 1.5 (logical 1280x720); the
Samsung panels (2560x1440, 700x400 mm) have PPI 92.54 → auto = 1.

### 4.5 Worked examples

Replicated in C++ against the real `libhyprutils` `Vector2D` (verified twice):

| Mode | Requested | Effective | Path | Logical (t = 0) |
|---|---|---|---|---|
| 2560x1440 | 1.25 | 1.25 | valid | 2048x1152 |
| 2560x1440 | 2 | 2 | valid | 1280x720 |
| 2560x1440 | 1.5 | 1.6 | notification | 1600x900 (raw 1599.999976) |
| 2560x1440 | 1.333 / 1.333333 | 4/3 | silent | 1920x1080 |
| 2560x1440 | 1.6 / 1.666667 | 1.6 / 5/3 | silent | 1600x900 / 1536x864 |
| 2560x1440 | 1.75 / 1.8 | 1.666667 | notification | 1536x864 |
| 2560x1440 | 1.1 | 1.066667 | notification | 2400x1350 |
| 2560x1440 | 1.2 | 1.25 | notification | 2048x1152 |
| 1920x1080 | 1.25 / 1.5 | same | valid | 1536x864 / 1280x720 |
| 1920x1080 | 1.2 | 1.2 | silent | 1600x900 |
| 1920x1080 | 1.75 | 1.666667 | notification | 1152x648 |
| 1920x1080 | 1.8 | 1.875 | notification | 1024x576 |
| 2560x1440, transform 1 | 1 | 1 | valid | 1440x2560 |

`1.6f` stored as float gives a raw quotient of 1599.99997; the final `m_size`
relies on `round()`. hyprtilt should offer only scales `k/120` for which both
axes divide exactly, written with enough decimals (e.g. `"1.666667"`), which
then snap silently.

## 5. Caelestia loading of hypr-user.lua; package.path, require, dofile, top-level return

### 5.1 Lua runtime

- The maintainer's `/usr/bin/Hyprland` 0.56.2 links **PUC-Rio Lua 5.5**
  (`DT_NEEDED liblua.so.5.5`, Arch `lua 5.5.1-1`); `liblua5.4` in `ldd` comes
  from `libinput.so.10`. Not LuaJIT (*live*, `readelf -d`).
- CMake does **not** strictly require 5.5:
  `pkg_search_module(LUA REQUIRED IMPORTED_TARGET GLOBAL lua55 lua5.5 lua-55 lua-5.5 lua>=5.5 lua<5.6)`
  takes the first matching alternative, and `lua<5.6` alone could match a 5.4
  `lua.pc` on another distro (whether that compiles is untested)
  ([CMakeLists.txt L291](https://github.com/hyprwm/Hyprland/blob/v0.56.2/CMakeLists.txt#L291)).
  Generate Lua in the 5.4/5.5 common subset.
- Every state is `luaL_newstate()` + `luaL_openlibs()` (all standard
  libraries), then `debug.sethook` and `debug.gethook` are set to `nil`
  ([ConfigManager.cpp L513-529](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L513-L529);
  `/usr/include/lualib.h` L62).
- Lua 5.5 reserves `global` unless built with `LUA_COMPAT_GLOBAL`; Arch's
  `luaconf.h` L353-354 defines it (*live*). Never emit `global` as a name.
- `hl` is a plain global set before any config code runs
  ([LuaBindingsRegistration.cpp L96](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsRegistration.cpp#L96));
  modules loaded by `require`, `dofile` or `loadfile` get the global table as
  `_ENV` ([Lua 5.5 manual §2.2](https://www.lua.org/manual/5.5/manual.html#2.2))
  and see `hl`; `local`s of the main file are not visible to them.

### 5.2 package.path

```cpp
// src/config/lua/ConfigManager.cpp L534-546 (v0.56.2)
    std::filesystem::path configDir = std::filesystem::path(m_mainConfigPath).parent_path();
    const std::string     luaPath   = (configDir / "?.lua").string() + ";" + (configDir / "?/init.lua").string();
    lua_getglobal(m_lua, "package");
    lua_getfield(m_lua, -1, "path");
    std::string combinedLuaPath = luaPath;
    if (const auto* originalLuaPath = lua_tostring(m_lua, -1); originalLuaPath && *originalLuaPath) {
        combinedLuaPath += ';';
        combinedLuaPath += originalLuaPath;
    }
    lua_pop(m_lua, 1);
    lua_pushlstring(m_lua, combinedLuaPath.data(), combinedLuaPath.size());
    lua_setfield(m_lua, -2, "path");
    lua_pop(m_lua, 1);
```

([ConfigManager.cpp L534-546](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L534-L546);
since 5ba33f84 / v0.55.0; tests
[LuaBindingsInternal.cpp L450-474](https://github.com/hyprwm/Hyprland/blob/v0.56.2/tests/config/lua/LuaBindingsInternal.cpp#L450-L474)).
`configDir` is the main config's directory, not canonicalised for XDG
discovery (canonical for `-c`). The default part comes from
`LUA_PATH_5_5`/`LUA_PATH` or `luaconf.h`; on Arch it ends with
`./?.lua;./?/init.lua` (*live*, `lua5.5 -e 'print(package.path)'`).

### 5.3 require

- The global `require` is replaced by the C closure `safeLuaRequire`; the
  original is kept as global `__require`. `package.searchers[2]` is wrapped
  so that every file it resolves is recorded in `m_configPaths` (for
  autoreload)
  ([ConfigManager.cpp L548-597](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L548-L597)).
- Explicit paths starting with `/`, `./`, `../` or `~/` are resolved relative
  to the **main config file** (not the calling file), `~` via `$HOME`;
  candidates `X`, `X.lua` (unless `X` ends in `.lua`) and `X/init.lua`. Paths
  containing `*`, `?` or `[` are globbed and return a table; their parent
  directory is watched as well
  ([ConfigManager.cpp L77-165](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L77-L165),
  [MiscFunctions.cpp L61-83](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/helpers/MiscFunctions.cpp#L61-L83)).
- Errors: `safeLuaRequire` runs the original `require` under `lua_pcall`.
  An error starting with `module '<name>' not found` is re-raised
  (`luaL_error`, aborting the calling chunk). Any other error (syntax error
  while loading, runtime error while running) is recorded as
  `require("<name>"): <msg>`, the path is still tracked,
  `package.loaded[name]` is set to an empty table and `require` returns it, so
  the caller continues
  ([ConfigManager.cpp L312-377](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L312-L377)).
  A missing *nested* module inside `hypr-user.lua` does not match the prefix
  for `hypr-user`, so it becomes a config error of `require("hypr-user")`.
- Within one load, `require` caches as in plain Lua; across reloads nothing is
  cached (§2.3).

### 5.4 dofile and loadfile

`dofile`/`loadfile` are the standard functions; they do not pass through
`package.searchers[2]`, so their files are not tracked for autoreload
(inference from the hook code,
[ConfigManager.cpp L561-597](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L561-L597)).
hyprmoncfg uses a guarded `dofile` (Appendix B).

### 5.5 Return values

The main chunk runs with `nresults = 0`, so anything it returns is discarded
([ConfigManager.cpp L746-751](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L746-L751)).
A runtime error in the main chunk is caught with a `luaL_traceback` handler,
so its `configerrors` entry spans several lines
([L740-744](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L740-L744)).

### 5.6 Caelestia

| Line of `hypr/hyprland.lua` | What happens | Source |
|---|---|---|
| L1-3 | `package.path = package.path .. ";" .. home .. "/.config/caelestia/?.lua"` (appended = lowest priority) | [L1-3](https://github.com/caelestia-dots/caelestia/blob/d459e9182ae55ba7eb29fa90781297224911d6bd/hypr/hyprland.lua#L1-L3) |
| L43-47 | `require("hypr-vars")` (its returned table **is** used, merged into `variables`), possibly `require("variables")` | [L43-76](https://github.com/caelestia-dots/caelestia/blob/d459e9182ae55ba7eb29fa90781297224911d6bd/hypr/hyprland.lua#L43-L76) |
| L53-59 | catch-all `hl.monitor({output = "", mode = "preferred", position = "auto", scale = 1})` | [L53-59](https://github.com/caelestia-dots/caelestia/blob/d459e9182ae55ba7eb29fa90781297224911d6bd/hypr/hyprland.lua#L53-L59) |
| L62-72 | `require` of the `hyprland.*` modules | [L43-76](https://github.com/caelestia-dots/caelestia/blob/d459e9182ae55ba7eb29fa90781297224911d6bd/hypr/hyprland.lua#L43-L76) |
| L75-76 | `maybe_create(home .. "/.config/caelestia/hypr-user.lua")` (creates an empty file if missing), then `require("hypr-user")` as the **last statement**, result discarded | same |

- The `return {}` at the end of the maintainer's `hypr-user.lua` (L31) is
  ignored; the comment saying so is the maintainer's own (L6-7), not
  Caelestia's. Caelestia's README only says the file is "loaded at the end of
  the Hyprland loading sequence"
  ([README.md L92-98](https://github.com/caelestia-dots/caelestia/blob/d459e9182ae55ba7eb29fa90781297224911d6bd/README.md#L92-L98)).
- Nothing in Caelestia's `hypr/` tree runs after `require("hypr-user")` that
  touches monitor rules; the `hl.on` handlers in `execs.lua` only exec
  commands and resize windows. What the Caelestia shell (quickshell) does over
  IPC was not checked. The live `~/.config/hypr` equals the repository except
  the generated `scheme/current.{lua,conf}` (*live*, `diff -r`).
- Effective search order for `require("hypr-user")` on the live session
  (Hyprland pid 3436: argv `Hyprland --watchdog-fd 4`, cwd `/home/mokulanis`,
  no `HYPRLAND_CONFIG`, `LUA_PATH*`, `XDG_CONFIG_HOME`): `~/.config/hypr/hypr-user.lua`,
  `~/.config/hypr/hypr-user/init.lua`, the Lua 5.5 default path including
  `./hypr-user.lua` relative to Hyprland's cwd, and only then
  `~/.config/caelestia/hypr-user.lua` (*live*, `/proc/3436`). No shadowing file
  exists today; `doctor` should check with `package.searchpath` semantics.

### 5.7 Top-level return

`return` may only be the last statement of a block
([Lua 5.5 manual §3.3.4](https://www.lua.org/manual/5.5/manual.html#3.3.4)).
Appending code after `return {...}` fails:
`luac5.5: appended.lua:4: <eof> expected near 'hl'` (exit 1), while the same
block inserted before the `return` compiles (*live*, `luac5.5 -p`).
The managed block goes before the last top-level `return`; find it with a
Lua tokenizer, not a regex.

### 5.8 Failure modes of the target file

| Error | Result on the next reload | Source |
|---|---|---|
| Syntax error in the **main** file | phase 1 fails, all old rules stay, one error | [ConfigManager.cpp L682-696](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L682-L696) |
| Syntax error in `hypr-user.lua` (e.g. code after `return`) | rules cleared; `require` fails with `error loading module 'hypr-user' from file '<path>':` / `<path>:<line>: <eof> expected near 'hl'` (two `configerrors` entries); only Caelestia's `output = ""` rule remains → every monitor preferred / auto / scale 1 / transform 0 (**needs runtime confirmation** for the visible result) | [ConfigManager.cpp L312-377](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L312-L377), [MonitorRuleManager.cpp L96-106](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRuleManager.cpp#L96-L106) |
| Runtime error part-way through `hypr-user.lua` | `hl.monitor` calls before the error stay; later ones are lost | same |
| Runtime error in the main chunk before `require("hypr-user")` | `hypr-user` never runs, is not tracked and not watched | [ConfigManager.cpp L561-597](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L561-L597) |
| `hypr-user` not found | `module 'hypr-user' not found` re-raised, main chunk aborts at its last line | [ConfigManager.cpp L312-377](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L312-L377) |

hyprtilt must validate the complete new file with a Lua 5.5 parser
(`luac5.5 -p`, or `luaL_loadbuffer` in an embedded Lua 5.5) before writing
it, and keep a backup.

## 6. Lua config availability by version and distribution

### 6.1 By Hyprland version

| Version | Lua config | Fresh install generates | `.lua` preferred over `.conf` | Notes | Source |
|---|---|---|---|---|---|
| ≤ 0.54.x | no (no `src/config/lua` at v0.54.3) | `hyprland.conf` | — | hyprlang only | [v0.54.3 src/config](https://github.com/hyprwm/Hyprland/tree/v0.54.3/src/config) |
| 0.55.0-0.55.2 | yes (5ba33f84) | **`hyprland.conf`** (fallback called `fullConfigPath` without `"lua"`); safe mode `recoverycfg.conf` | yes (f61ee4c2) | no `reload full-reset`, no `repl` | [v0.55.2 Jeremy.cpp L35](https://github.com/hyprwm/Hyprland/blob/v0.55.2/src/config/supplementary/jeremy/Jeremy.cpp#L35), [hyprutils Path.cpp L9-15](https://github.com/hyprwm/hyprutils/blob/v0.14.1/src/path/Path.cpp#L9-L15) |
| 0.55.3-0.55.4 | yes | `hyprland.lua` (backport 44a70fe9); `recoverycfg.lua` | yes | `full-reset` backported (84ccaf3d) | [commit 44a70fe9](https://github.com/hyprwm/Hyprland/commit/44a70fe9) |
| 0.56.0 | yes | `hyprland.lua` (61321ce5, #14944) | yes | tagged on `main`; `repl`, `full-reset` (e43b163f), `hl.monitor` changes (§1.6) | [commit 61321ce5](https://github.com/hyprwm/Hyprland/commit/61321ce5) |
| 0.56.1-0.56.2 | yes | `hyprland.lua` | yes | release branch; any `.conf` config shows `You are using the .conf config format, support for which will be removed in Hyprland 0.57.` (de88bcaf, #15538) | [i18n/Engine.cpp L254](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/i18n/Engine.cpp#L254), [Compositor.cpp L900-901](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/Compositor.cpp#L900-L901) |
| `main` (untagged, reports `0.56.0`) | Lua only | `hyprland.lua`; with only a `.conf` present, a default `.lua` is generated and the `.conf` is silently ignored; `-c foo.conf` is parsed as Lua | — | a9902ea6 (#15539, 2026-07-22) removed legacy | [commit a9902ea6](https://github.com/hyprwm/Hyprland/commit/a9902ea65f461af868497c4e74fed798ade0b1b0), [main VERSION](https://github.com/hyprwm/Hyprland/blob/4bb6844b0351e4fbf2e3d4e46ae71b551a0e0a42/VERSION) |

Version dates: v0.55.0 is 2026-05-09 (lightweight tag, commit date); v0.56.0
2026-07-20 (on `main`; the merge-base of v0.56.2 and `main` is 36b2e0cf
"version: bump to 0.56.0"); v0.56.2 2026-08-05. `git tag --contains a9902ea6` is
empty and v0.56.1/v0.56.2 are not ancestors of `main` (Hyprland git history,
[compare](https://github.com/hyprwm/Hyprland/compare/v0.56.2...main)).

### 6.2 How Hyprland 0.56.2 picks the config file

1. Safe mode → `<instance>/recoverycfg.lua`.
2. `-c`/`--config` → `std::filesystem::canonical` (symlinks resolved), must be a
   regular file, else exit 1
   ([main.cpp L119-142](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/main.cpp#L119-L142), [L264](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/main.cpp#L264)).
3. `$HYPRLAND_CONFIG`, used raw.
4. First existing `hypr/hyprland.lua` (`hyprlandd` in debug builds) in
   `$XDG_CONFIG_HOME` (only if set and absolute), `$HOME/.config` (if `HOME` is
   absolute), each `$XDG_CONFIG_DIRS` entry, then `/etc/xdg`.
5. Only then the first `hypr/hyprland.conf` in the same order.
6. Otherwise `<XDG_CONFIG_HOME or ~/.config>/hypr/hyprland.lua`, generated.

([Jeremy.cpp L19-54](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/supplementary/jeremy/Jeremy.cpp#L19-L54),
[hyprutils Path.cpp L59-93](https://github.com/hyprwm/hyprutils/blob/v0.14.1/src/path/Path.cpp#L59-L93).)
So `/etc/xdg/hypr/hyprland.lua` beats `~/.config/hypr/hyprland.conf`. The
manager is chosen by extension: `.lua` → Lua, anything else → legacy
([config/ConfigManager.cpp L29-68](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/ConfigManager.cpp#L29-L68)).
The path is cached in a function-static (env values cached for the process);
plain `reload` never switches, `reload full-reset` re-resolves it and rebuilds
the manager
([Jeremy.cpp L47-52](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/supplementary/jeremy/Jeremy.cpp#L47-L52),
[HyprCtl.cpp L1260-1279](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L1260-L1279)).
No `hyprctl` request exposes the main path; read `/proc/<pid>/cmdline` and
`/proc/<pid>/environ` (exec-time only) of the pid from `hyprctl instances -j`,
or the early log lines (§10.5).

### 6.3 By distribution (all accessed 2026-09-28)

| Distribution | Release | Hyprland | Lua | `monitorv2` | Source |
|---|---|---|---|---|---|
| Arch Linux | extra | 0.56.2-3 | yes | yes | [archlinux.org JSON](https://archlinux.org/packages/extra/x86_64/hyprland/json/) |
| openSUSE | Factory / Tumbleweed | 0.56.2 | yes | yes | [OBS spec](https://api.opensuse.org/public/source/openSUSE:Factory/hyprland/hyprland.spec) |
| NixOS | unstable / 26.05 / 25.11 | 0.56.2 / 0.55.4 / 0.52.2 | yes / yes / no | yes | [nixpkgs package.nix](https://github.com/NixOS/nixpkgs/blob/nixos-unstable/pkgs/by-name/hy/hyprland/package.nix) (per branch) |
| Debian | 12 bookworm | none | — | — | [packages.debian.org](https://packages.debian.org/bookworm/hyprland) |
| Debian | 13 trixie main | none | — | — | [packages.debian.org](https://packages.debian.org/trixie/hyprland) |
| Debian | 13 trixie-backports (opt-in) | 0.55.2+ds-1~bpo13+1 | yes, but a fresh install generates `.conf` | yes | [madison](https://api.ftp-master.debian.org/madison?package=hyprland&text=on) |
| Debian | forky (testing), sid | 0.56.2+ds-3 (autoremoval from testing scheduled 2026-10-29) | yes | yes | [tracker](https://tracker.debian.org/pkg/hyprland) |
| Ubuntu | 22.04, 24.04 | none | — | — | [packages.ubuntu.com](https://packages.ubuntu.com/noble/hyprland) |
| Ubuntu | 24.10, 25.04, 25.10 (all Obsolete) | 0.41.2 | no | no | [Launchpad](https://launchpad.net/ubuntu/+source/hyprland/+publishinghistory) |
| Ubuntu | 26.04 LTS (current stable) | 0.53.3+ds-4 | no | yes (with `sdr_eotf`, without `icc`) | [Launchpad API](https://api.launchpad.net/1.0/ubuntu/+archive/primary?ws.op=getPublishedSources&source_name=hyprland&exact_match=true&status=Published) |
| Ubuntu | 26.10 (pre-release freeze) | 0.56.2+ds-1 | yes | yes | same |
| Fedora | 42 (archived, EOL 2026-05-27) / 43 / rawhide / 44 | 0.45.2 / retired / retired / none | no | no | [packages.fedoraproject.org](https://packages.fedoraproject.org/pkgs/hyprland/hyprland/), [Bodhi F42](https://bodhi.fedoraproject.org/releases/F42) |
| Fedora COPR | lionheartp/Hyprland (wiki-recommended) | 0.56.2-3 (fedora-44, 45, rawhide) | yes | yes | [COPR API](https://copr.fedorainfracloud.org/api_3/package?ownername=lionheartp&projectname=Hyprland&packagename=hyprland&with_latest_succeeded_build=true) |
| Fedora COPR | sdegler/hyprland | 0.56.2-2 (fedora-43..45, rawhide) | yes | yes | [COPR API](https://copr.fedorainfracloud.org/api_3/package?ownername=sdegler&projectname=hyprland&packagename=hyprland&with_latest_succeeded_build=true) |
| Fedora COPR | solopasha/hyprland (abandoned) | 0.49.0-7 (2025-10-05) | no | no | [COPR API](https://copr.fedorainfracloud.org/api_3/package?ownername=solopasha&projectname=hyprland&packagename=hyprland&with_latest_succeeded_build=true) |
| Alpine | edge, v3.24 / v3.23 / v3.22 | 0.54.3 / 0.51.1 / 0.49.0 | no | yes / yes / no | [pkgs.alpinelinux.org](https://pkgs.alpinelinux.org/packages?name=hyprland) |

The wiki calls Ubuntu's package "extremely outdated" and still lists 24.10
([installation.md L202-221](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/getting-started/installation.md)).
Not surveyed: Gentoo, Void, Ubuntu PPAs, Debian/Ubuntu derivatives (§13).

### 6.4 Choosing the backend

- Decide by the file Hyprland would load (§6.2) and confirm with
  `j/status` → `configProvider` (0.55+). A missing `configProvider` means
  < 0.55, i.e. hyprlang.
- Hyprland 0.55+ still loads an existing `hyprland.conf` when no
  `hyprland.lua` exists, so upgraded Arch users can be on hyprlang until 0.57.
- Git builds of `main` report `0.56.0` but have no hyprlang: never infer
  "hyprlang works" from a version below 0.57.
- With nothing to detect (offline snippet), Ubuntu/Debian stable-like systems
  imply hyprlang (§6.3).
- On Nix (home-manager, Hjem) `hyprland.lua` or the whole `~/.config/hypr`
  is often a read-only store link; in-place editing must detect this and
  refuse ([wiki Hjem](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/nix/configuring-hyprland-with-hjem.md)).

## 7. hyprctl version -j

Fields ([SystemInfo.cpp L99-141](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/helpers/SystemInfo.cpp#L99-L141)), with *live* values:

| Field | Type | Live value / meaning |
|---|---|---|
| `branch` | string | `v0.56.2` |
| `commit` | string | `efb50993780079460b0cbed1363e2166a2de1d9f` |
| `version` | string | `0.56.2` (from the `VERSION` file) |
| `dirty` | bool | `false` |
| `commit_message`, `commit_date` | string | |
| `tag` | string | `v0.56.2` (`git describe --tags`; else `$GIT_TAG`; else `unknown`) |
| `commits` | **string** | `"7661"` |
| `buildAquamarine`, `buildHyprlang`, `buildHyprutils`, `buildHyprcursor`, `buildHyprgraphics` | string | build-time versions (`buildHyprlang` 0.6.8, `buildHyprutils` 0.14.1) |
| `systemAquamarine`, `systemHyprlang`, `systemHyprutils`, `systemHyprcursor`, `systemHyprgraphics` | string | runtime versions (`systemHyprutils` 0.14.2) |
| `abiHash` | string | |
| `flags` | array of string | subset of `debug`, `no xwayland`, `nix`; live `[]` |

- `main`'s `VERSION` still says `0.56.0` and its `tag` is e.g.
  `v0.56.0-209-g4bb6844b`
  ([main VERSION](https://github.com/hyprwm/Hyprland/blob/4bb6844b0351e4fbf2e3d4e46ae71b551a0e0a42/VERSION),
  [CMakeLists.txt L202-205, L245-247](https://github.com/hyprwm/Hyprland/blob/v0.56.2/CMakeLists.txt#L202-L205)).
- Version gates used in this document: Lua ≥ 0.55.0; `reload full-reset`
  ≥ 0.55.3; `repl` ≥ 0.56.0; `monitorv2` ≥ 0.50.0; its `sdr_eotf` ≥ 0.52.0
  (string form ≥ 0.54.0); `icc` ≥ 0.55.0; `desc:` prefix match ≥ 0.47.0;
  `desc:` trimming ≥ 0.49.0.

## 8. hyprlang monitor= and monitorv2 syntax

Applies only to the legacy manager in Hyprland ≤ 0.56.x (§6).

### 8.1 monitor=

`monitor` is an unscoped handler registered without flags
([legacy ConfigManager.cpp L606](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L606)),
so it also fires inside any category block
([hyprlang config.cpp L839-873](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L839-L873)).
The value is split by `CVarList2`: delimiter `,`, every argument trimmed,
`\,` escapes a comma, empty arguments kept, out-of-range index = `""`
([hyprutils VarList2.cpp L35-103](https://github.com/hyprwm/hyprutils/blob/v0.14.1/src/string/VarList2.cpp#L35-L103)).
hyprlang's own escape pass turns `\\` into `\` first, so both `\,` and `\\,`
reach it as an escaped comma
([hyprlang config.cpp L808-827](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L808-L827)).

```text
line     = "monitor" "=" SEL "," rest
rest     = "disable" | "disabled"
         | "addreserved" "," TOP "," BOTTOM "," LEFT "," RIGHT
         | "transform" "," N
         | MODE "," POSITION "," SCALE { "," KEYWORD "," VALUE }
KEYWORD  = "mirror" | "bitdepth" | "cm" | "sdrsaturation" | "sdrbrightness"
         | "transform" | "vrr" | "icc"            (exact, case-sensitive)
```

([legacy ConfigManager.cpp L1293-1387](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1293-L1387))

| Form | Behaviour | Source |
|---|---|---|
| `SEL,MODE,POSITION,SCALE,...` | fresh `CMonitorRuleParser` (**no merge**); a missing field is `""` = preferred / auto / auto; `add()` replaces any rule with the same selector string | [L1345-1386](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1345-L1386) |
| unknown `KEYWORD` | returns `invalid syntax at "<kw>"` **before** `add()`: the whole line has no effect, including valid extras parsed earlier. Applies to `sdr_eotf`, `supports_*`, `*_luminance` and typos | [L1375-1379](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1375-L1379) |
| value errors (mode, position, scale, extras) | appended to `m_error` (each with a trailing space), rule **added** with fallbacks, message returned | [L1384-1386](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1384-L1386), [Parser.cpp L104-108](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L104-L108) |
| empty argument at a keyword position | extras parsing stops silently (`DP-1,preferred,auto,1,,vrr,1` ignores `vrr`); a value slot is consumed even when empty; a keyword in the last position reads `""` as its value; a trailing comma adds no empty argument | [L1351](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1351), [VarList2.cpp L86-93](https://github.com/hyprwm/hyprutils/blob/v0.14.1/src/string/VarList2.cpp#L86-L93) |
| `SEL,disable` / `SEL,disabled` | adds a default rule with only `m_disabled = true`; erases an earlier rule with the same string; further arguments ignored | [L1299-1342](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1299-L1342) |
| `SEL,addreserved,T,B,L,R` | order top, bottom, left, right → `CReservedArea(top, right, bottom, left)`, negatives clamped to 0; `stoi` failure → `parse error: invalid reserved area`. Patches an existing same-string rule in place (not moved). With none, falls through and adds a bare default rule **without** the area (**needs runtime confirmation**) | [L1319-1342](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1319-L1342), [ReservedArea.cpp L14-17](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/desktop/reserved/ReservedArea.cpp#L14-L17) |
| `SEL,transform,N` | copies the existing same-string rule with the new transform and re-`add()`s it (moves it to the end, changing precedence against other selectors); silent no-op if none exists; bad N → `invalid transform ` | [L1302-1318](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1302-L1318) |

Extras ([Parser.cpp L213-290](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L213-L290)):

| Keyword | Value | Error |
|---|---|---|
| `mirror` | any string, unvalidated | none |
| `bitdepth` | exactly `"10"` → 10-bit; anything else 8-bit | none |
| `cm` | `auto`, `srgb`, `wide`, `edid`, `hdr`, `hdredid`, `dcip3`, `dp3`, `adobe` (case-sensitive) | `invalid cm ` |
| `sdrsaturation`, `sdrbrightness` | `stof` (a numeric prefix is enough) | `invalid sdrsaturation ` / `invalid sdrbrightness ` |
| `transform` | integer string (no float) in 0..7 | `invalid transform `; ERR log `Invalid transform {} in monitor` only when out of range |
| `vrr` | integer string (no float); < 0 = unset | `invalid vrr ` |
| `icc` | non-empty string (≥ 0.55) | `invalid icc ` |

`parseTransform` and `parseVRR` call `std::stoi` after `isNumber` without
`try`; a digit string that overflows `int` would throw out of the handler
(**needs runtime confirmation**); range-check before writing
([Parser.cpp L213-273](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L213-L273)).
MODE, POSITION and SCALE use the same parsers as Lua (§1.2, §12).

### 8.2 monitorv2

```text
monitorv2 {
    output = desc:Samsung Electric Company Odyssey G50F HNAYC01389   # must be the first assignment
    mode = 2560x1440@179.95
    position = 3360x975
    scale = 1
    vrr = 2
}
monitorv2[DP-1]:transform = 1                                        # inline form
```

- Keyed special category with key `output`
  ([legacy ConfigManager.cpp L567](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L567)).
  A first assignment other than `output` gives
  `special category's first value must be the key. Key for <monitorv2> is <output>`
  (only that line is reported) ([hyprlang config.cpp L291-438](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L291-L438);
  [wiki hyprlang.md L101-107](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/hypr-ecosystem/dev/hyprlang.md)).
- Rules are built only in `reload()` **after the whole file is parsed**
  (`handleMonitorv2()` has one caller). Every `monitorv2` rule is therefore
  added after all `monitor=` rules: it replaces a `monitor=` rule with the same
  string wherever each sits, and wins against other selectors for the same
  monitor; an `addreserved` aimed at a `monitorv2`-only selector has no effect
  (**needs runtime confirmation**)
  ([L726-751](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L726-L751)).
- Blocks are processed in first-appearance order; the first block that
  returns an error stops all later blocks (the erroring block itself is still
  added when the error is a value error). The error has no file or line and is
  shown only if the hyprlang parse had no error
  ([L881-891](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L881-L891),
  [L747-750](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L747-L750)).
- Only fields with `m_bSetByUser` are applied; unset fields keep the
  `CMonitorRule` defaults, not the registered ones
  ([L788-879](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L788-L879)).
- INT fields use hyprlang `configStringToInt`: decimal, `0x` hex,
  `rgba(...)`/`rgb(...)`, and any value **starting with** `true`/`on`/`yes`
  (→ 1) or `false`/`off`/`no` (→ 0), so `nope` → 0. FLOAT failures give
  `failed parsing a float: ...`
  ([hyprlang config.cpp L185-274](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L185-L274)).

| Field | hyprlang type | Registered default | Effective when unset | Parsing | Lua key |
|---|---|---|---|---|---|
| `disabled` | INT | 0 | false | raw | `disabled` |
| `mode` | STR | `preferred` | preferred | `parseMode` | `mode` |
| `position` | STR | `auto` | auto | `parsePosition` | `position` |
| `scale` | STR | `auto` | auto (-1) | `parseScale` | `scale` |
| `addreserved` | STR | `[[EMPTY]]` | none | `"T,B,L,R"` via `CVarList` + `stoi`; failure → `parse error: invalid reserved area`, no rule | `reserved` / `reserved_area` |
| `mirror` | STR | `[[EMPTY]]` | none | `setMirror`, unvalidated | `mirror` |
| `bitdepth` | STR | `[[EMPTY]]` | 8-bit | `"10"` → 10-bit | `bitdepth` (int) |
| `cm` | STR | `auto` | **`srgb`** | `parseCM` | `cm` |
| `sdr_eotf` | STR | `default` | default | `"0"` → auto, `"1"` → srgb, `"2"` → gamma22, else `fromString` (`"3"` → srgb; unknown → default) | `sdr_eotf` (digits mean something else) |
| `sdrbrightness` | FLOAT | 1.0 | 1.0 | raw, no range | `sdrbrightness` |
| `sdrsaturation` | FLOAT | 1.0 | 1.0 | raw | `sdrsaturation` |
| `vrr` | INT | 0 | **unset** (`misc:vrr`) | < 0 → unset | `vrr` |
| `transform` | STR | `[[EMPTY]]` | 0 | `parseTransform` | `transform` (int) |
| `supports_wide_color` | INT | 0 | 0 | raw | same |
| `supports_hdr` | INT | 0 | 0 | raw | same |
| `sdr_min_luminance` | FLOAT | 0.2 | 0.2 | raw | same |
| `sdr_max_luminance` | INT | 80 | 80 | raw | same |
| `min_luminance` | FLOAT | -1.0 | -1 | raw | same |
| `max_luminance` | INT | -1 | -1 | raw | same |
| `max_avg_luminance` | INT | -1 | -1 | raw | same |
| `icc` | STR | `""` | none | raw | `icc` |

(`[[EMPTY]]` is `STRVAL_EMPTY`; registrations at
[L567-588](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L567-L588);
effective defaults at [MonitorRule.hpp L41-69](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L41-L69).)
The numeric `sdr_eotf` values differ from `fromString`'s table (`"0"`
default, `"1"` gamma22, `"2"` gamma22force); write names
([TransferFunction.cpp L10-19](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/helpers/TransferFunction.cpp#L10-L19)).

Version history of `monitorv2`: introduced by abdfc5ea (#9761, 2025-06-05),
first tag v0.50.0 (none at v0.49.0); HDR fields c3894d92 (#10623), also
v0.50.0; `sdr_eotf` added by ff50dc36 (#12094) in v0.52.0 as `INT{0}`,
string `{"default"}` since v0.54.0; `icc` (also as a `monitor=` extra) since
v0.55.0 (absent at v0.54.3). Unknown keys in a special category are config
errors, so gate `sdr_eotf` and `icc` by version
([commit abdfc5ea](https://github.com/hyprwm/Hyprland/commit/abdfc5ea40c0793117f34bf6c465840500cf7cb0),
[commit ff50dc36](https://github.com/hyprwm/Hyprland/commit/ff50dc36),
[v0.50.0 ConfigManager.cpp L800-819](https://github.com/hyprwm/Hyprland/blob/v0.50.0/src/config/ConfigManager.cpp#L800-L819)).
Two blocks with the same `output` appear to merge into one category (open
question, §13). Emit plain `monitor=` lines by default (every version accepts
them) and `monitorv2` only for fields `monitor=` cannot express, on ≥ 0.50.

### 8.3 hyprlang text rules relevant to a textual scanner

| Rule | Detail | Source |
|---|---|---|
| Line joining | a line ending in `\` is joined with the next (whitespace before `\` stripped) **before** comment handling: a comment ending in `\` swallows the next line; `\` on the last line → `Last line ends with backslash`. Never end generated lines with `\`, and check the line before the block | [config.cpp L44-65](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L44-L65) |
| Comments | line trimmed; `#` at position 0 = comment (`# hyprlang ...` = directive; a line starting `##` is still a comment); elsewhere `#` cuts the rest unless doubled, `##` = literal `#`; no `=` → must end with `{` or be `}`; split at the first `=`, both sides trimmed; empty LHS → `Empty lhs.` | [config.cpp L669-728](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L669-L728), [wiki hyprlang.md L72-77](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/hypr-ecosystem/dev/hyprlang.md) |
| Variables | `$NAME = v` defines or overwrites; expansion is raw substring replacement of `$NAME` in the RHS (and LHS of non-variable lines), up to 100 passes; `{{a op b}}` single-operator arithmetic inside each pass. **Environment variables are included**: every parse seeds the list from `environ`, so `$HOME` etc. expand in `monitor=` lines; the list is sorted longest-first only when a new variable is added | [config.cpp L730-806](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L730-L806), [L1070-1076](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L1070-L1076), [L518-527](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L518-L527) |
| Directives | `# hyprlang if VAR` / `if !VAR` ... `endif` (VAR truthy = exists in env or config and non-empty; env searched first; nesting not propagated), `# hyprlang noerror [true/false]`; directives are processed even inside a failed `if` | [config.cpp L553-617](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L553-L617), [L676-684](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L676-L684) |
| Version var | Hyprland exports `HYPRLAND_V_0_53=1` only while parsing, usable in `# hyprlang if` and as `$HYPRLAND_V_0_53` | [legacy ConfigManager.cpp L710-745](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L710-L745) |
| `source = PATH` | `glob(GLOB_TILDE)` on `absolutePath(PATH, currentFile)`; relative to the current file's directory (one `../` level resolved); each regular file parsed in place and appended to `m_configPaths`; errors `source= globbing error: found no match`, `source= path <p> bogus!` (< 2 chars), `source= file <p> is inaccessible!`; rules from sourced files interleave in parse order | [legacy ConfigManager.cpp L1802-1855](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L1802-L1855) |
| Errors | `throwAllErrors = true`: parsing continues; each error is `Config error in file <path> at line <n>: <msg>`; `<path>` is `canonical()` for the main file (symlinks resolved), the glob result for sourced files; `<n>` is the first physical line of a joined line; a sourced file's errors are repeated as the error of its `source=` line; unclosed `{` → `Unclosed category at EOF` | [legacy ConfigManager.cpp L486](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L486), [config.cpp L942-944](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L942-L944), [L1004-1056](https://github.com/hyprwm/hyprlang/blob/v0.6.8/src/config.cpp#L1004-L1056) |
| Config dump | legacy `systeminfo -c` prints `Config File: <path>: Read Succeeded` plus content for each file | [legacy ConfigManager.cpp L687-704](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/legacy/ConfigManager.cpp#L687-L704) |

Treat user lines outside the managed block as opaque (they may contain
`$vars`, `##`, continuations).

## 9. Output selectors

```cpp
// src/output/Monitor.cpp L1201-1211 (v0.56.2)
bool CMonitor::matchesStaticSelector(std::string_view selector) const {
    if (selector.starts_with("desc:")) {
        // match by description
        const auto DESCRIPTIONSELECTOR = trim(selector.substr(5));

        return m_description.starts_with(DESCRIPTIONSELECTOR) || m_shortDescription.starts_with(DESCRIPTIONSELECTOR);
    } else {
        // match by selector
        return m_name == selector;
    }
}
```

([Monitor.cpp L1201-1211](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L1201-L1211); identical on main at L1172-1182.)

### 9.1 Forms

| Selector | Matches | Notes |
|---|---|---|
| `DP-1` (any string not starting with `desc:`) | `m_name` exactly | no wildcard, no regex |
| `desc:<text>` | `trim(<text>)` is a prefix of `m_description` or of `m_shortDescription` | `m_description` = backend description with commas erased; `m_shortDescription` = trim(`"{make} {model} {serial}"`) with commas erased ([Monitor.cpp L250-256](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L250-L256)) |
| `""` | never matches in the lookup | used only as the fallback rule (§11) |
| `desc:` (empty after trim) | every monitor (`starts_with("")`) | never emit |

### 9.2 Pitfalls

- The selector text is trimmed but **not** comma-stripped: a selector with a
  comma never matches. `hyprctl monitors -j` `description` is
  `m_shortDescription`, which never contains commas
  ([HyprCtl.cpp L277](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L277)).
- Prefix matching makes a shortened selector match every monitor of that
  make and model. *Live*: HDMI-A-1 is
  `Samsung Electric Company Odyssey G50F HNAYC01385`, DP-1 is
  `Samsung Electric Company Odyssey G50F HNAYC01389`;
  `desc:Samsung Electric Company Odyssey G50F` matches both, and the
  last-added rule wins for both. eDP-1 has an empty
  serial (`Najing CEC Panda FPD Technology CO. ltd 0x004D`): a `Make Model`
  description is a prefix of `Make Model SERIAL` of a same-model monitor.
  `doctor` should flag `desc:` rules matching more than one connected monitor.
- Write `desc:` + the exact hyprctl `description`, no space after the colon.
- In hyprlang a `#` must be written `##` and `$WORD` may be expanded as a
  variable (§8.3); hyprmoncfg falls back to the connector name for `$`, `,`,
  CR, LF but not `#` ([render.go L505-508](https://github.com/crmne/hyprmoncfg/blob/ff88b554f5b150fc05077cfab94255686a65ed49/internal/render/render.go#L505-L508)).
- A Lua `output = 1` is accepted and becomes `"1"`, which matches nothing
  unless a connector is named `1` (§1.2).

### 9.3 History

| Version | `desc:` behaviour | Source |
|---|---|---|
| v0.15.1beta | introduced (71e2562a, 2022-10-05), exact match | [commit](https://github.com/hyprwm/Hyprland/commit/71e2562a4151b506cece69f7d121847957d4bac4) |
| v0.15.1beta-v0.46.x | exact match (from v0.37.0: `== szShortDescription` or `== szDescription`; Ubuntu's 0.41.2 is in this range) | [v0.41.2 Monitor.cpp L390-394](https://github.com/hyprwm/Hyprland/blob/v0.41.2/src/helpers/Monitor.cpp#L390-L394) |
| v0.47.0 | prefix match (2e2e2e2c, fixes #8756) | [commit](https://github.com/hyprwm/Hyprland/commit/2e2e2e2cad97eb017ab02f8a67b751e0abe3bb72) |
| v0.49.0 | text after `desc:` trimmed (da2d7c39, #9788) | [commit](https://github.com/hyprwm/Hyprland/commit/da2d7c3971d40f841f2afd7def8e4bad9a351e41) |

Writing the full hyprctl description therefore works on every packaged
version (0.41.2 builds `szShortDescription` the same way).

### 9.4 Mirror targets

`mirror` is resolved at apply time by
`monitorState()->query().relativeTo(focused).configString(mirrorOf)`, which
accepts `current`, a direction letter, relative `+N`/`-N`, a numeric monitor
ID, and finally `matchesStaticSelector` (connector name or `desc:` prefix).
Mirroring a mirror or itself logs `Cannot mirror a mirror!` /
`Cannot mirror self!`; an unresolvable target silently disables mirroring
([Monitor.cpp L1334-1399](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L1334-L1399),
[MonitorQueryCore.cpp L196-262](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/state/MonitorQueryCore.cpp#L196-L262)).
Write connector names.

## 10. Verifying that a file is loaded

### 10.1 What Hyprland exposes

- No `hyprctl` request and no Lua binding lists loaded files.
  `getConfigPaths()` is used only by the file watcher; `systeminfo -c` under
  Lua prints `Not supported under lua`
  ([ConfigWatcher.cpp L34](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/inotify/ConfigWatcher.cpp#L34),
  [ConfigManager.cpp L1069-1079](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L1069-L1079),
  [HyprCtl.cpp L1092-1106](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/debug/HyprCtl.cpp#L1092-L1106)).
- **inotify watches** (Linux): for each fd of the Hyprland process pointing to
  `anon_inode:inotify`, `/proc/<pid>/fdinfo/<fd>` lists
  `inotify wd:.. ino:.. sdev:.. mask:..`; match `ino` against `stat` of the
  candidate file. *Live*:
  `ino:a158fc` = `~/.config/caelestia/hypr-user.lua`, `a158fa` =
  `hypr-vars.lua`, `9e222a` = `~/.config/hypr/hyprland.lua`, `mask:8`
  (`IN_CLOSE_WRITE`). Caveats: on btrfs `sdev` (0x1d) differs from `st_dev`
  (0x32), so match by inode; the list is empty with `misc:disable_autoreload`;
  a module that failed to compile is still tracked; after a main-file syntax
  error the list shrinks to the main file (§10.2).
- **`configerrors -j`**: `[""]` = no errors (§3.4). Attribute entries by
  prefix: `<abs path>:<line>: hl.monitor: ...` or `require("<module>"): ...`
  (§1.4, §5.3). Tracebacks span several entries. Read it only right after a
  reload: `eval` and Lua-mode `dispatch` clear it (§2.2); errors raised outside
  parse or eval go to notifications only.
- **Live state**: compare `monitors all -j` with the block (mode, position,
  effective scale, transform). A mismatch may come from a wlr-output-management
  client (§11.4) or scale snapping (§4), not from the file not being loaded.
  `vrr` and `cm` cannot be compared (§3.4). *Live*: the three `hypr-user.lua`
  rules match the running state (HDMI-A-1 transform 1 at 0,0; eDP-1 at
  1440,1335; DP-1 179.952 Hz at 3360,975).
- **`configreloaded`** does not mean success (§3.6).
- **`status`**: `configProvider` tells Lua from hyprlang (§3.4).

### 10.2 Autoreload

- The watcher watches every path in `m_configPaths` (main file plus every file
  resolved by `require`, including modules that failed to compile but not
  "not found" ones). Files: `IN_CLOSE_WRITE | IN_DONT_FOLLOW`; a symlinked
  path additionally gets `IN_CLOSE_WRITE` on its canonical target; wildcard
  parent directories also get create/delete/move events. **Any** event on a
  known watch descriptor calls `reload()` synchronously, once per event read
  (several events → several reloads). The list is rebuilt after every reload
  (`REFRESH_ALL` includes the watcher) and emptied by
  `misc:disable_autoreload = true` (default `false`, *live* `false`)
  ([ConfigWatcher.cpp L32-111](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/inotify/ConfigWatcher.cpp#L32-L111),
  [ConfigManager.cpp L416-419](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L416-L419),
  [ConfigValues.cpp L492](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/values/ConfigValues.cpp#L492)).
- A plain write + close of `hypr-user.lua` reloads immediately; a half-written
  file can be picked up.
- An atomic temp-file + `rename()` produces no `IN_CLOSE_WRITE` on the watched
  inode; the `IN_IGNORED` of the dropped inode reaches the callback, which does
  not filter on the mask. Whether this reloads is **needs runtime
  confirmation**. Renaming over a symlinked target replaces the link (dotfile
  managers) with a regular file.
- After a reload whose main file fails phase 1, `m_configPaths` is only the
  main file, so edits to `hypr-user.lua` stop autoreloading until the main file
  loads again ([ConfigManager.cpp L643-696](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L643-L696)).
- Therefore: write, then send an explicit `reload` (a double reload is
  harmless) and wait for its reply.

### 10.3 Suggested doctor sequence

1. Resolve the main file as in §6.2 and the target with `package.searchpath`
   semantics over Hyprland's `package.path` (§5.2, §5.6).
2. Confirm `configProvider` (§3.4).
3. After hyprtilt's own `reload` (reply received): read `configerrors -j`,
   filter to the target path.
4. Check the inotify watch list for the target inode.
5. Compare `monitors all -j` with the block; if different, suspect an
   output-management client.

`repl package.searchpath("hypr-user", package.path)` would return the
resolved file from the live state, but it executes code and clears
`configerrors` ([ConfigManager.cpp L880-953](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L880-L953)).

### 10.4 Log file

`$XDG_RUNTIME_DIR/hypr/<instance>/hyprland.log`: `debug:disable_logs`
defaults to `true`, so Hyprland's own lines stop after early startup (live:
~18 of ~12000 lines are Hyprland's; the rest are aquamarine). The early lines
still name the main config: `[cfg] Regular config at <path>` and
`[cfg] Using lua config found at <path>`, or
`[cfg] Config is either explicit or special.`
for `-c`/`HYPRLAND_CONFIG`
([ConfigValues.cpp L629](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/values/ConfigValues.cpp#L629); *live*).

## 11. Rule order and precedence

### 11.1 Storage

At most one rule per exact output string: `add()` erases same-string rules
and appends (§1.5). Lua calls merge into the previous same-string rule; legacy
`monitor=` lines replace it; legacy `transform` short form copies and
re-appends; `addreserved` patches in place (§8.1)
([MonitorRuleManager.cpp L37-42](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRuleManager.cpp#L37-L42)).

### 11.2 Lookup

```cpp
// src/config/shared/monitor/MonitorRuleManager.cpp L86-116 (v0.56.2), CMonitorRuleManager::get
    if (PMONITOR->m_isUnsafeFallback) {
        CMonitorRule fallbackRule;
        fallbackRule.m_autoDir    = DIR_AUTO_RIGHT;
        fallbackRule.m_name       = PMONITOR->m_name;
        fallbackRule.m_resolution = Vector2D{1920, 1080};
        fallbackRule.m_offset     = Vector2D{-INT32_MAX, -INT32_MAX};
        fallbackRule.m_scale      = 1;
        return fallbackRule;
    }

    for (auto const& r : m_rules | std::views::reverse) {
        if (PMONITOR->matchesStaticSelector(r.m_name))
            return applyWlrOutputConfig(r);
    }

    Log::logger->log(Log::WARN, "No rule found for {}, trying to use the first.", PMONITOR->m_name);

    for (auto const& r : m_rules) {
        if (r.m_name.empty())
            return applyWlrOutputConfig(r);
    }

    Log::logger->log(Log::WARN, "No rules configured. Using the default hardcoded one.");

    CMonitorRule fallbackRule;
    fallbackRule.m_autoDir    = eAutoDirs::DIR_AUTO_RIGHT;
    fallbackRule.m_name       = "";
    fallbackRule.m_resolution = Vector2D{};
    fallbackRule.m_offset     = Vector2D{-INT32_MAX, -INT32_MAX};
    fallbackRule.m_scale      = -1;
    return applyWlrOutputConfig(fallbackRule);
```

([MonitorRuleManager.cpp L86-116](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRuleManager.cpp#L86-L116))

1. An unsafe-fallback monitor gets a fixed 1920x1080, scale 1 rule (no wlr override).
2. Otherwise the **last-added** rule whose selector matches wins (reverse
   iteration), not the most specific one.
3. Otherwise the **first** rule with an empty output string.
4. Otherwise a hardcoded preferred / auto-right / auto-scale rule.

A specific rule therefore always beats `""` wherever each appears, and
Caelestia's `output = ""` never overrides a user rule. Two different selector
strings matching one monitor (`DP-1` and `desc:...`) are separate rules and
the later one wins in full; merging happens only between identical strings.
The wiki documents only the fallback behaviour
([monitors/_index.md L35](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/configuring/core/monitors/_index.md)).

### 11.3 Order of rule creation

- Lua: rules are created in execution order (main file, then modules as
  `require` runs them); Caelestia: `""` at L53-59, then `hyprland.*`, then
  `hypr-user` last (§5.6). A managed block in `hypr-user.lua` comes after
  every Caelestia rule.
- hyprlang: parse order across `source=`d files, then all `monitorv2` blocks
  (§8.2), then `keyword` rules at runtime (§2.4).
- Rules are applied only after the whole config ran (`postConfigReload`); only
  the final set matters
  ([ConfigManager.cpp L809-818](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/ConfigManager.cpp#L809-L818)).
- Placement rule for hyprtilt: the managed block must come after any other
  rule for the same monitor (or reuse its exact selector and write every
  field), and in hyprlang no user `monitorv2` block may target the same
  monitor.

### 11.4 wlr-output-management overrides

`get()` passes the chosen rule through `applyWlrOutputConfig`: if a
wlr-output-management client (kanshi, wlr-randr, nwg-displays, wdisplays,
shikane) committed state for that connector name, `m_disabled` is overridden
unconditionally and mode/refresh, position, transform, scale and adaptive sync
per committed property
([MonitorRuleManager.cpp L45-84](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRuleManager.cpp#L45-L84)).
The state is stored per client `COutputManager` and erased only when that
client's `zwlr_output_manager_v1` is destroyed; no monitor re-apply is
scheduled then, and config reloads do not clear it
([OutputManagement.cpp L644-653](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/protocols/OutputManagement.cpp#L644-L653),
[L598-600](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/protocols/OutputManagement.cpp#L598-L600)).
A running daemon keeps overriding config edits; a one-shot tool stops
overriding when it exits but its state stays until the next
`ensureMonitorStatus` (**needs runtime confirmation**).

## 12. Geometry

### 12.1 Transforms

The value is cast directly to `wl_output_transform` and mapped 1:1 to
hyprutils `eTransform`
([LuaBindingsConfigRules.cpp L103-107](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/lua/bindings/LuaBindingsConfigRules.cpp#L103-L107),
[Math.cpp L22-35](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/helpers/math/Math.cpp#L22-L35)).
Protocol text from `/usr/share/wayland/wayland.xml` (wayland 1.26.0, L2923-2944):
"The flipped values correspond to an initial flip around a vertical axis
followed by rotation", applied to buffer contents during presentation. The
wiki gives no direction
([positioning.md L83-90](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/configuring/core/monitors/positioning.md)).

| Value | Protocol entry | Protocol meaning | Wiki text | Swaps W/H |
|---|---|---|---|---|
| 0 | `normal` | no transform | normal (no transforms) | no |
| 1 | `90` | 90° counter-clockwise | 90 degrees | yes |
| 2 | `180` | 180° | 180 degrees | no |
| 3 | `270` | 270° counter-clockwise | 270 degrees | yes |
| 4 | `flipped` | 180° flip around a vertical axis | flipped | no |
| 5 | `flipped_90` | flip, then rotate 90° counter-clockwise | flipped + 90 degrees | yes |
| 6 | `flipped_180` | flip, then rotate 180° | flipped + 180 degrees | no |
| 7 | `flipped_270` | flip, then rotate 270° counter-clockwise | flipped + 270 degrees | yes |

Width and height swap exactly for odd values
([Monitor.cpp L1058](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L1058)).
Proposed TUI mapping (inference from the protocol text, **unverifiable
without a runtime test**): transform 1 compensates a panel physically turned
90° clockwise; "rotate right" `t → (t & 4) | ((t + 1) & 3)`, "rotate left"
`t → (t & 4) | ((t + 3) & 3)`, "flip" `t → t ^ 4`. Confirm on the maintainer's
portrait HDMI-A-1 (transform 1).

Lua requires an integer literal (`transform = 1`, §1.3). Legacy
`parseTransform`: a non-integer string only appends `invalid transform `; an
integer outside 0..7 also logs `Invalid transform {} in monitor`
([Parser.cpp L213-226](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L213-L226)).

### 12.2 Positions and arrangement

- Positions are **logical** (scaled and transformed) coordinates; the layout
  box is `{m_position, m_size}` and an explicit position is the rule offset
  verbatim; the wiki agrees (a 4K monitor at scale 2 is 1920 wide, 1080 if
  rotated)
  ([Monitor.cpp L1249-1254](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L1249-L1254),
  [L1768-1770](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L1768-L1770),
  [positioning.md L12-13](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/configuring/core/monitors/positioning.md)).
- Syntax: §1.2 and
  [Parser.cpp L145-191](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L145-L191);
  `stoi` ignores trailing garbage (`"0x0abc"`) and truncates fractions;
  invalid `auto*` logs a WARN.
- Arrangement
  ([MonitorPositionController.cpp L16-101](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/state/MonitorPositionController.cpp#L16-L101),
  [MonitorLayoutController.cpp L55-67](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/state/MonitorLayoutController.cpp#L55-L67)):
  monitors with an explicit offset are moved verbatim first, in any config
  order. Auto monitors follow in `monitors()` order (order of `monitor.added`;
  a re-enabled or un-mirrored monitor is appended again), each placed against
  the bounding box of placed monitors; the integer accumulators start at 0, so
  the box always contains the origin. right/none → `(maxRight, 0)`; left →
  `(minLeft - w, 0)`; up → `(0, minUp - h)`; down → `(0, maxDown)`;
  center-up/down centre x over the box width; center-left/right centre y over
  the box height, computed in `double` (half pixels possible, hyprctl truncates).
  Golden cases from upstream tests: auto-left next to a 100x100 box at the
  origin → `(-50, 0)`, auto-up → `(0, -50)`, auto-center-right 50x40 →
  `(100, 30)`, auto-center-down 40x50 → `(30, 100)`
  ([tests/state/MonitorPositionController.cpp L162-220](https://github.com/hyprwm/Hyprland/blob/v0.56.2/tests/state/MonitorPositionController.cpp#L162-L220)).
- The first monitor is not special-cased (wiki disagreement above).
- Overlaps: monitors are added to a `CRegion` in turn; the first intersecting
  one gets ERR `Monitor {}: detected overlap with layout` and a 15000 ms
  notification
  (`Your monitor layout is set up incorrectly. Monitor {name} overlaps with other monitor(s) in the layout. ...`),
  then the loop breaks.
  Positions are not changed
  ([MonitorLayoutController.cpp L39-53](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/state/MonitorLayoutController.cpp#L39-L53),
  [i18n/Engine.cpp L245-247](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/i18n/Engine.cpp#L245-L247)).
  Replicated with the real hyprutils `CRegion`/pixman: exact edge contact and
  corner contact are not overlaps; 1 px, 0.5 px and 0.4 px overlaps are.
  Keep all positions integral.
- No code closes gaps or snaps edges
  ([MonitorPositionController.cpp L24-29](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/state/MonitorPositionController.cpp#L24-L29)).
  Snapping, adjacency and overlap checks are hyprtilt's job.
- Disabled monitors turn the output off early in `applyMonitorRule` (no mode
  or scale work) and are erased from `monitors()` via `monitor.removed`;
  mirrors copy the mirrored monitor's position once at `setMirror` time and are
  erased from `monitors()` on every `layoutChanged`, but still apply their own
  mode and scale. Neither takes part in arrangement or overlap checks
  ([Monitor.cpp L731-744](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L731-L744),
  [MonitorState.cpp L29-46](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/state/MonitorState.cpp#L29-L46),
  [Monitor.cpp L1340-1390](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L1340-L1390)).

Maintainer layout (*live* `monitors all -j`, `hypr-user.lua` L5-10):

| Output | Mode | Transform | Scale | Position | Logical size | x range | y range |
|---|---|---|---|---|---|---|---|
| HDMI-A-1 | 2560x1440@144 | 1 | 1 | 0x0 | 1440x2560 | 0-1440 | 0-2560 |
| eDP-1 | 1920x1080@144 | 0 | 1 | 1440x1335 | 1920x1080 | 1440-3360 | 1335-2415 |
| DP-1 | 2560x1440@179.952 | 0 | 1 | 3360x975 | 2560x1440 | 3360-5920 | 975-2415 |

Neighbours share edges exactly with no overlap and no horizontal gap. eDP-1
and DP-1 bottoms align at y = 2415, but HDMI-A-1 ends at 2560 (145 px lower),
although the config comment says the bottom edges are aligned; aligning would
need HDMI-A-1 at y = -145. Good golden test.

### 12.3 Mode matching

- `mode` grammar: §1.2. A modeline is recognised only if its first
  whitespace token equals `modeline` case-insensitively and at least 9 numeric
  arguments parse (`CVarList2` on spaces keeps empty fields, so a double space
  breaks it); a failed modeline falls through to `WxH` parsing and normally
  yields `invalid resolution ` → preferred
  ([Parser.cpp L14-143](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L14-L143)).
  `"1920x1080@120Hz"` parses because `stof` stops at `H`.
- Without `@`, `m_refreshRate` is **not assigned**: a fresh rule keeps the
  default **60 Hz**; a merged Lua rule keeps the previous call's refresh
  ([MonitorRule.hpp L46](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.hpp#L46)).
  *Live*: HDMI-A-1 offers `2560x1440@144.00/120.00/59.95Hz`;
  `mode = "2560x1440"` would select 59.95 Hz. Keywords `preferred`, `highrr`,
  `highres`, `maxwidth` also leave it unchanged.
- Explicit `WxH@Hz`: available modes are sorted by closeness (smaller `|dx|`,
  then `|dy|`, then `|mHz/1000 - Hz|`) and the best 3 kept. If the best is not
  within 1 px on both axes **and** within 1 Hz (`DELTALESSTHAN`, strict `<`),
  the requested mode is added as a custom mode and tried first. A
  `DRM_MODE_TYPE_USERDEF` modeline is tried before everything. Test order:
  modeline, custom request, best1, best2, best3, preferred; the first that
  passes `m_state.test()` wins
  ([Monitor.cpp L818-916](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L818-L916),
  [macros.hpp L40](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/macros.hpp#L40)).
  Two decimals as shown in `availableModes` are enough: `@179.95` selects the
  live 179.952 Hz mode without a custom mode.
- Named modes: `preferred` → preferred mode, then the first 3 listed modes;
  `highrr` → sort by `round(mHz)` descending, ties (equal mHz only) broken by
  being larger on both axes; `highres` → strictly larger on both axes, then
  refresh; `maxwidth` → widest, then refresh; best 3 each; preferred is always
  the last fallback
  ([Monitor.cpp L759-817](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L759-L817)).
- If nothing passes: (1) the requested resolution as a custom mode, except
  for preferred/highrr/highres (`maxwidth` is tried with `(-1,-3)`); refresh 0
  on non-DRM backends; (2) every available mode, the first working one with a
  WARN and a 5000 ms notification
  `Monitor {name} failed to set any requested modes, falling back to mode {mode}.`;
  (3) no mode: ERR `Monitor {} has NO FALLBACK MODES...` and up to 3 retries
  1 s apart
  ([Monitor.cpp L919-969](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L919-L969),
  [L1084-1093](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L1084-L1093),
  [i18n/Engine.cpp L248](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/i18n/Engine.cpp#L248)).
- Modelines:
  `modeline <clock MHz> <hdisplay> <hsync_start> <hsync_end> <htotal> <vdisplay> <vsync_start> <vsync_end> <vtotal> [flags]`.
  The clock is
  parsed as float and assigned to `uint32_t` before `*= 1000`, so sub-MHz
  digits are lost (1071.101 → 1071000 kHz); `vrefresh` is stored in mHz. Flags
  are lowercased before lookup but the map key is `"Interlace"`, so interlace
  never matches (`Invalid flag interlace in modeline`); only `+hsync`,
  `-hsync`, `+vsync`, `-vsync` work. Modelines are honoured only on the DRM
  backend (`Tried to set custom modeline on non-DRM output`). Round-trip a
  user's modeline string verbatim
  ([Parser.cpp L48-87](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/Parser.cpp#L48-L87),
  [Monitor.cpp L843-848](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/output/Monitor.cpp#L843-L848),
  `/usr/include/xf86drmMode.h` L91-101,
  [wiki modes.md](https://github.com/hyprwm/hyprland-wiki/blob/d5498a9876fc00d7324d74a7c19a8e8473bad207/content/configuring/core/monitors/modes.md)).
  **main:** only logging and `format_to_n` for the mode name changed.

### 12.4 Modeset versus soft apply

Position, transform, mirror, reserved area, colour management and auto
direction changes apply without a modeset; resolution, refresh, scale,
bitdepth, modeline and enable changes do a full modeset (§2.2). The
comparison uses configured values, so a corrected scale does not cause
repeated re-applies
([MonitorRule.cpp L9-31](https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/config/shared/monitor/MonitorRule.cpp#L9-L31)).

## 13. Open questions and facts that need runtime confirmation

None of these could be tested because experiments that change compositor
state were not allowed.

1. Physical direction of transform 1 and therefore of the TUI rotate keys
   (§12.1); confirm on HDMI-A-1.
2. Does an atomic temp-file + `rename()` of `hypr-user.lua` trigger autoreload
   (via `IN_IGNORED`), or is the watch silently lost (§10.2)?
3. Does a syntax error in `hypr-user.lua` visibly revert all three monitors to
   Caelestia's `output = ""` rule on the next reload (§5.8)? Source says yes.
4. Stale modeline carry-over in merged Lua rules, including a failed modeline
   leaving partial `USERDEF` data (§1.5).
5. Latency of an `eval`'d `hl.monitor` (next rendered frame); may it never
   apply on an idle or VT-switched session (§2.2)?
6. VRR-only changes not applied by `eval` or reload because `compare` ignores
   `m_vrr` (§2.2).
7. Does `monitors all -j` show the new state as soon as the `reload` reply
   arrives, or do DRM commits land later (§2.3)?
8. Does a request of exactly N x 1023 bytes, or one split across writes, hang
   or truncate the v0.56.2 socket (§3.3)?
9. Lifetime of wlr-output-management overrides after the client exits, and
   whether anything re-applies config rules then (§11.4).
10. Order of `monitoradded` versus mode application when a rule re-enables a
    monitor (`onConnect(true)`) (§3.6).
11. How `lua_tostring` formats a float `scale` such as `1.0666666666667`
    before `stof`, i.e. whether numeric scales lose 1/120 precision (§1.3).
12. Legacy `monitor=SEL,addreserved,...` with no earlier rule adds a bare rule
    without the area (§8.1).
13. A `monitorv2` block always overrides a `monitor=` line with the same
    selector regardless of order, and `addreserved` against a
    `monitorv2`-only selector has no effect (§8.2).
14. `hyprctl keyword monitorv2[DP-1]:mode ...` has no effect until reload,
    which then discards it (§2.4).
15. Two `monitorv2` blocks with the same `output` merge into one category,
    later values winning (hyprlang config.cpp L372-374) (§8.2).
16. `transform,99999999999` / `vrr,...` overflow throwing out of the legacy
    handler (§8.1).
17. `hl.env("HYPRLAND_CONFIG", ...)` followed by `reload full-reset` switches
    the config file (wiki claim).
18. What Aquamarine does with a custom `WxH@Hz` mode that is not in the EDID
    list (CVT timings?); the Aquamarine source was not examined.
19. Does the ErrorOverlay `failed to find a clean divisor` message appear in
    `configerrors`? Not traced.
20. How auto-positioned monitor order interacts with hotplug and reload order.
21. Is HDMI-A-1's bottom edge at 2560 (145 px below the others) intended
    (§12.2)? Question for the maintainer.
22. Whether a CMake build that resolves `lua<5.6` to Lua 5.4 compiles (§5.1).
23. `hyprctl status -j` JSON output (source only; not on the allowed command
    list) (§3.4).
24. Release timing and contents of Hyprland 0.57 (expected to drop hyprlang,
    as announced by 0.56.1+).
25. Ubuntu 26.10's final Hyprland version; whether Debian testing keeps 0.56.2
    after the 2026-10-29 autoremoval date; Gentoo, Void, PPAs and derivatives
    not surveyed (§6.3).
26. Whether HyprMon's Lua writer is in a tagged HyprMon release (only HEAD
    `32ad27f9` verified) (Appendix B).

## Appendix A. hyprtilt's own examples against these facts

The hero example in [`docs/index.md`](index.md) omits `scale`. Because a
fresh rule's scale is auto (§1.2) and the eDP-1 panel's PPI is 143.66, that
block would give eDP-1 scale 1.5 (logical 1280x720) instead of the live 1
(§4.4), and DP-1 at x = 3360 would no longer touch it. Examples should write
`scale` explicitly.

## Appendix B. Other monitor tools

| Tool | Revision | What it writes | Load mechanism | Live apply | Source |
|---|---|---|---|---|---|
| nwg-displays (GUI, 0.4.4) | `fd79522c` (2026-08-24) | overwrites `monitors.conf` (default `~/.config/hypr/monitors.conf`, `-m`) and sibling `monitors.lua` (`hl.monitor({...})`) on every apply; optional `desc:{description}`; escapes `#` as `##` in `.conf` only | user adds `source = ...` or `require("monitors")` | `hyprctl reload`, `hyprctl dispatch dpms` | [settings_applier.py L454-541](https://github.com/nwg-piotr/nwg-displays/blob/fd79522cb91ef2ba080ba51838c0f307e1d4fb83/nwg_displays/settings_applier/settings_applier.py#L454-L541), [README L87-112](https://github.com/nwg-piotr/nwg-displays/blob/fd79522cb91ef2ba080ba51838c0f307e1d4fb83/README.md#L87-L112) |
| hyprmon (Go TUI) | `32ad27f9` (2026-07-31) | Lua: `hyprmon.lua` next to the resolved config; hyprlang: rewrites `monitor=` lines in `hyprland.conf` in place; backups `<config>.bak.<unix time>`; optional `desc:` | appends `-- hyprmon: managed monitor profile include` + `require("hyprmon")` to the root file | `hyprctl keyword monitor ...` with connector names | [hyprland.go L443-487, L700-765](https://github.com/erans/hyprmon/blob/32ad27f94f40fd0dd81f982cc8208bb615607202/hyprland.go#L443-L487) |
| hyprmoncfg (Go TUI) | `ff88b554` (2026-09-28) | owns `~/.config/hypr/hyprmoncfg-monitors.lua` (or `.conf`); `monitorv2` when `hyprctl version` ≥ 0.50.0; connector-name fallback for `desc:` with `$`, `,`, CR, LF | one line appended to the root config; for Lua a guarded `dofile` | — | [README L121](https://github.com/crmne/hyprmoncfg/blob/ff88b554f5b150fc05077cfab94255686a65ed49/README.md#L121), [client.go L185-191](https://github.com/crmne/hyprmoncfg/blob/ff88b554f5b150fc05077cfab94255686a65ed49/internal/hypr/client.go#L185-L191), `include.go` L188-216 in [the repository at ff88b554](https://github.com/crmne/hyprmoncfg/tree/ff88b554f5b150fc05077cfab94255686a65ed49) (directory not recorded) |
| HyprDynamicMonitors | `ce4292c0` (2026-02-02) | hyprlang `~/.config/hypr/monitors.conf` only, no Lua | `source =` | — | [README L116-121](https://github.com/fiffeek/hyprdynamicmonitors/blob/ce4292c0b136c7933eaf3b79ac72e659d81323cb/README.md#L116-L121) |

None of them edits `hl.monitor` calls in place inside the user's own Lua file;
all write a sidecar (hyprmon edits in place only for legacy `.conf`)
(inference from the sources above). A sidecar loaded last overrides earlier
rules for the same monitor, while an in-place block must sit after any other
rule for that monitor (§11.3). Appending a load line after a top-level
`return` (as the sidecar tools do to the root file) would break a file like
`hypr-user.lua` (§5.7).
