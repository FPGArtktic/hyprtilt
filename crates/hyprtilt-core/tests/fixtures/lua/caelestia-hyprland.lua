-- A reduced version of the main configuration of a Caelestia-style setup:
-- it extends package.path and loads the user's file last with require.
local home   = os.getenv("HOME")
package.path = package.path .. ";" .. home .. "/.config/caelestia/?.lua"

-- Default monitor rule for outputs without a rule of their own.
hl.monitor({
    output   = "",
    mode     = "preferred",
    position = "auto",
    scale    = 1,
})

require("hyprland.general")
require("hyprland.keybinds")

-- User configs
require("hypr-user")
