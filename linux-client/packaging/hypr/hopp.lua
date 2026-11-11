-- Hopp for Omarchy: window rules and keybindings.
-- Installed by `make install` as ~/.config/hypr/hopp.lua and loaded with
-- require("hypr.hopp") from hyprland.lua. `make uninstall` removes both.

local hopp = @BINARY@

-- Call window: small, floating, on every workspace, never in your screen share,
-- and it never steals focus when a call starts. Its size comes from the app.
o.window("^hopp\\.faces$", {
  float = true,
  pin = true,
  no_initial_focus = true,
  no_screen_share = true,
  move = { "(monitor_w-window_w-10)", "(monitor_h-window_h-10)" },
})

-- Frosted glass: the call window and the bands around a shared screen are see-through,
-- and Hyprland blurs the desktop behind them. Omarchy ships with blur off; turning it on
-- only changes windows that are see-through, and none of Omarchy's are by default.
hl.config({ decoration = { blur = { enabled = true, size = 10, passes = 3 } } })

-- The shared screen is a normal tile (no rule needed). Peek puts the whole screen
-- in its own special workspace, over whatever you're doing.
o.window("^hopp\\.peek$", { workspace = "special:hopp" })

-- The Hopp window floats like an Omarchy panel instead of taking a tile.
o.window("^hopp$", {
  float = true,
  center = true,
  size = { 420, 640 },
})

-- Settings float the same way, at the size the window asks for.
o.window("^hopp\\.settings$", {
  float = true,
  center = true,
  size = { 860, 620 },
})

-- Incoming-call and agent-card fallback windows when layer-shell is unavailable.
o.window("^hopp\\.call$", { float = true, pin = true, no_initial_focus = true })
o.window("^hopp\\.cards$", { float = true, pin = true, no_initial_focus = true, no_screen_share = true })

-- The call card, agents' cards, the "you're sharing" pill, others' marks over your
-- shared monitor, the pad you draw on it with and the reactions (their tray and the
-- layer they play on) are for you, not for your viewers (they draw their own marks and
-- play their own reactions; upstream's Mac and Windows apps do too).
hl.layer_rule({ match = { namespace = "^hopp-(call|sharing|cards|marks|draw|reactions)$" }, no_screen_share = true })
-- Hyprland hides those by painting black boxes over them in the share, which for the
-- full-screen marks and reactions layers and the draw pad would black out the whole
-- screen. The hopp-share-clean plugin (hyprland-plugin/ in Hopp) leaves them out of the
-- share instead; without it Hopp doesn't open them over the shared monitor. No
-- fade-out, so a closing overlay never flashes into the share.
hl.layer_rule({ match = { namespace = "^hopp-(marks|draw|reactions)$" }, no_anim = true })
@SHARE_PLUGIN@

o.bind("SUPER + ALT + H", "Hopp", hopp .. " panel")
o.bind("SUPER + H", "Hopp: peek at the shared screen", hopp .. " peek")
o.bind("SUPER + CTRL + ESCAPE", "Hopp: revoke remote control", hopp .. " revoke")
o.bind("SUPER + ALT + C", "Hopp: capture the call, or stop", hopp .. " capture toggle")
o.bind("SUPER + ALT + D", "Hopp: draw on the shared screen, or stop", hopp .. " draw")
o.bind("SUPER + ALT + P", "Hopp: stamp a note on the shared screen, or stop", hopp .. " stamp")
-- Call controls from anywhere, as the Mac app's global shortcuts. Super+Alt+S is
-- Omarchy's scratchpad, so sharing is X. Rebind any of these in your own Hyprland config.
o.bind("SUPER + ALT + M", "Hopp: mute or unmute", hopp .. " mute")
o.bind("SUPER + ALT + V", "Hopp: camera on or off", hopp .. " camera")
o.bind("SUPER + ALT + X", "Hopp: share your screen, or stop", hopp .. " share")
o.bind("SUPER + ALT + L", "Hopp: leave the call", hopp .. " leave")
