-- SPDX-License-Identifier: GPL-3.0-only
-- Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
--
-- The demo's copy of a Caelestia hypr-user.lua (see demo/README.md).

-- My own Hyprland settings. Caelestia never overwrites this file.
-- The files in ~/.config/hypr/ belong to Caelestia; do not edit them.

-- Monitor layout: [HDMI-A-1 portrait] [eDP-1 laptop] [DP-1]
hl.monitor({ output = "HDMI-A-1", mode = "2560x1440@144",    position = "0x0",       scale = 1, transform = 1 })
hl.monitor({ output = "eDP-1",    mode = "1920x1080@144",    position = "1440x1335", scale = 1 })
hl.monitor({ output = "DP-1",     mode = "2560x1440@179.95", position = "3360x975",  scale = 1, vrr = 2 })

-- Key repeat: the default delay was too short.
hl.config({
    input = {
        repeat_delay = 500,
        repeat_rate  = 30,
    },
})

return {

  -- Kept for later:
  -- env = {
  --   "LIBVA_DRIVER_NAME,nvidia",
  -- },
}
