# While you talk

A sound plays as rookey starts listening, stops, types and fails. There are three sets, made from tones and noise as they play, so nothing is shipped or licensed: `ROOKEY_SOUNDS=notes` (the default: soft sine tones), `rook` (a small caw, beak clacks, a keycap knock) and `pencil` (short taps of noise). Any cue can be your own file instead: `ROOKEY_SOUND_START`, `ROOKEY_SOUND_STOP`, `ROOKEY_SOUND_TYPED`, `ROOKEY_SOUND_FAILED`, each a path, played through `pw-play` or `paplay` (`afplay` on macOS; WAV only on Windows). `rookey ui` has all of it under While you talk, with a Play button for each. `ROOKEY_QUIET=1` turns the sounds off.

A small pill sits at the bottom of the screen while it listens (a level meter and the seconds so far), transcribes and types, and turns red with the reason if something failed. It's `rookey overlay`, which recordings start themselves: a layer-shell surface on Wayland (niri, Hyprland, sway, KDE), a borderless window on every Space on macOS, a layered window on Windows. It never takes a click or the focus, and it quits once rookey is idle.

- `ROOKEY_PILL=compact` drops the clock and the words, `ROOKEY_PILL=dot` shows one dot. A failure is the whole pill in every style.
- `ROOKEY_PILL_AT=x,y` moves it, in percent of the screen from the top left. `50,100`, the bottom centre, is the default.
- `ROOKEY_NO_OVERLAY=1` turns it off.

Where the pill is off or can't show (GNOME and X11 have no layer shell), a desktop notification says the same, failures included; `ROOKEY_NO_NOTIFICATIONS=1` turns those off too. Windows has no notifications, only the sounds.
