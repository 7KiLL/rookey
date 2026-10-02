# Hotkeys

There are two ways, and `rookey ui` keeps one at a time: with both, one press would start and stop the recording at once.

## rookey listens for it

The default. Hold the keys to talk and let go to stop. A tap shorter than 0.3 s keeps it recording until the next press. The keys go in `ROOKEY_HOTKEY`: a combination like `Super+Shift+D`, or a key on its own like `Control_R`, `Alt_R` or `F13`. Keys something else binds are reported first, with the line that binds them, and lone keys you press while typing (Shift, the left Ctrl, Alt, Super) are refused. `rookey ui` starts the listener at every login, and restarts it when the keys change.

| | How it hears the keys | What starts it | The window you are in gets the keys too |
|---|---|---|---|
| Linux | evdev, which sees a key go up again; compositor binds can't (niri has no release binds at all) | `~/.config/systemd/user/rookey-listen.service`, tied to `graphical-session.target` | Not on niri and Hyprland: `rookey ui` adds a bind there that does nothing. Elsewhere it does, and always for a modifier on its own |
| macOS | a keyboard tap, in Rookey | a launchd agent | No, except a modifier on its own |
| Windows | the keys' state, so it needs no rights of its own | your user's Run key, `rookey-listen` | Yes |

## Your desktop runs it

Bind `rookey toggle` in your compositor, or in anything else that runs a command. One press starts, the next one stops. `rookey ui` writes the bind where it can. Every change is checked by the compositor's own validator (`niri validate`, `Hyprland --verify-config`) and undone, byte for byte, if that finds a fault.

| Desktop | What `rookey ui` writes |
|---|---|
| niri 26.04+ | `rookey.kdl` next to your config, and one `include "rookey.kdl" optional=true` line in `user.kdl` if your config includes one, else in the main config |
| Hyprland | a marked block in `user.lua` if there is one, else in `hyprland.lua` (or `hyprland.conf` on a Hyprland older than 0.55) |
| older niri, macOS, others | nothing, the line to add is shown |

By hand:

```kdl
// niri, inside binds { }
Mod+Shift+D repeat=false { spawn "rookey" "toggle"; }
```
```lua
-- Hyprland
hl.bind("SUPER + SHIFT + D", hl.dsp.exec_cmd("rookey toggle"))
```

On macOS, bind it in skhd, Raycast or Shortcuts. The recording runs as Rookey, so the permissions it needs are Rookey's, not the hotkey app's.

## How the text gets typed

- **Linux**: `wtype` on niri, Hyprland, sway and COSMIC, which let it type. GNOME and KDE don't, so there it's pasted like on macOS: `wl-copy` puts the text on the clipboard and the primary selection, and a virtual keyboard presses Shift+Insert, which pastes in terminals too. The clipboard's text is put back 300 ms later (the primary selection keeps the dictation); `ROOKEY_KEEP_CLIPBOARD=0` leaves it. If nothing can be typed, `rookey status` says why.
- **macOS**: pasted with Cmd+V through System Events, because typing keys mangles anything that isn't ASCII. The clipboard's text is put back 300 ms later (an image or files on it are lost); `ROOKEY_KEEP_CLIPBOARD=0` leaves the typed text on it instead.
- **Windows**: Unicode key presses. A window running as administrator takes no input from rookey unless rookey runs as administrator too.
