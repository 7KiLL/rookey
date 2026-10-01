<p align="center">
  <a href="https://rookey.click">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/banner-dark.png">
      <img src="assets/banner-light.png" alt="rookey: say it, it's typed. Local dictation for Linux, macOS and Windows." width="100%">
    </picture>
  </a>
</p>

<p align="center">
  <a href="https://rookey.click"><b>rookey.click</b></a> ·
  <a href="https://github.com/7KiLL/rookey/releases/latest">Download</a> ·
  <a href="#hotkeys">Hotkeys</a> ·
  <a href="#engines">Engines</a> ·
  <a href="#settings">Settings</a> ·
  <a href="#when-something-goes-wrong">Troubleshooting</a>
</p>

<p align="center">
  <a href="https://github.com/7KiLL/rookey/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/7KiLL/rookey?style=flat-square&color=C0342B&label=release"></a>
  <img alt="Linux, macOS, Windows" src="https://img.shields.io/badge/runs_on-Linux_·_macOS_·_Windows-17231E?style=flat-square">
  <a href="LICENSE"><img alt="Apache-2.0" src="https://img.shields.io/badge/license-Apache--2.0-4A5B52?style=flat-square"></a>
  <img alt="Written in Rust" src="https://img.shields.io/badge/Rust-whisper.cpp-4A5B52?style=flat-square">
</p>

Hold a key, talk, let go: the text is typed into whatever window you are in. Whisper runs on your own machine, so nothing leaves it unless you pick a cloud engine. Filler words and false starts are dropped, the language is picked for you, and names on your screen, like `useSettingsStore`, come out spelled right.

## Install

```sh
curl -fsSL https://rookey.click/install | sh      # Linux, macOS
```
```powershell
irm https://rookey.click/install.ps1 | iex        # Windows, in PowerShell
```

Then, once:

```sh
rookey setup     # checks the mic, downloads a speech model (or takes an ElevenLabs key instead), sets the hotkey
```

The installers put the release build for your system on your PATH (`~/.local/bin`, or `%LOCALAPPDATA%\rookey` on Windows) and nothing else. The speech model (1.5 GB for the default) is downloaded only when you pick it in `rookey setup`, and not at all if you use ElevenLabs. Other builds, and building from source: [Builds](#builds).

What else each system needs:

| System | Needs |
|---|---|
| Linux | A Wayland desktop and `wtype`, which types the text. For hold to talk, read access to `/dev/input`: the `input` group, or an ACL from your login manager |
| macOS | Apple silicon. The settings page asks for the microphone, Accessibility (the hotkey and the typing), Automation of System Events (the paste) and, for screen terms, Screen Recording. macOS files them under *Rookey*, the small app rookey runs as, not under your terminal or your hotkey app |
| Windows | 10 or 11, nothing else. The CUDA build brings its own runtime |

For [screen terms](#screen-terms), `tesseract` on any system, and `grim` on Linux.

## Use

```
rookey            # record until Enter, print the transcript (pipe it: rookey | wl-copy)
rookey -v         # show the words as they arrive (-vv: every step with its time, -vvv: every audio chunk)
rookey toggle     # 1st call: start recording. 2nd call: stop, transcribe, type into the focused window
rookey listen     # hold ROOKEY_HOTKEY to talk, let go to stop (a tap keeps it going)
rookey setup      # the settings page, with what is missing first
rookey ui         # the settings page, in its own window (--browser for the browser, --no-open only prints the link)
rookey status     # what rookey is doing now, for bars (--json, --waybar, --follow)
rookey history    # the last transcripts, oldest first (--clear deletes them)
rookey update     # installs a newer release (--check only says whether there is one)
rookey skills     # a skill for coding agents: how to use rookey and find out why it failed (--install puts it in ~/.claude/skills)
rookey --help     # every command, and the settings with where they are saved on this system
```

Every transcript is kept before it is typed, so text that went into the wrong window, or wasn't typed at all, can be copied again: the last 500, as JSON lines in `history` in the [data folder](#settings), readable only by you and never uploaded. `rookey ui` lists them with a Copy button each. `ROOKEY_HISTORY=0` keeps none. Tests on the settings page aren't kept.

## Hotkeys

There are two ways, and `rookey ui` keeps one at a time: with both, one press would start and stop the recording at once.

### rookey listens for it

The default. Hold the keys to talk and let go to stop. A tap shorter than 0.3 s keeps it recording until the next press. The keys go in `ROOKEY_HOTKEY`: a combination like `Super+Shift+D`, or a key on its own like `Control_R`, `Alt_R` or `F13`. Keys something else binds are reported first, with the line that binds them, and lone keys you press while typing (Shift, the left Ctrl, Alt, Super) are refused. `rookey ui` starts the listener at every login, and restarts it when the keys change.

| | How it hears the keys | What starts it | The window you are in gets the keys too |
|---|---|---|---|
| Linux | evdev, which sees a key go up again; compositor binds can't (niri has no release binds at all) | `~/.config/systemd/user/rookey-listen.service`, tied to `graphical-session.target` | Not on niri and Hyprland: `rookey ui` adds a bind there that does nothing. Elsewhere it does, and always for a modifier on its own |
| macOS | a keyboard tap, in Rookey | a launchd agent | No, except a modifier on its own |
| Windows | the keys' state, so it needs no rights of its own | your user's Run key, `rookey-listen` | Yes |

### Your desktop runs it

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

### How the text gets typed

- **Linux**: `wtype`.
- **macOS**: pasted with Cmd+V through System Events, because typing keys mangles anything that isn't ASCII. The clipboard's text is put back 300 ms later (an image or files on it are lost); `ROOKEY_KEEP_CLIPBOARD=0` leaves the typed text on it instead.
- **Windows**: Unicode key presses. A window running as administrator takes no input from rookey unless rookey runs as administrator too.

## While you talk

A sound plays as rookey starts listening, stops, types and fails. There are three sets, made from tones and noise as they play, so nothing is shipped or licensed: `ROOKEY_SOUNDS=notes` (the default: soft sine tones), `rook` (a small caw, beak clacks, a keycap knock) and `pencil` (short taps of noise). Any cue can be your own file instead: `ROOKEY_SOUND_START`, `ROOKEY_SOUND_STOP`, `ROOKEY_SOUND_TYPED`, `ROOKEY_SOUND_FAILED`, each a path, played through `pw-play` or `paplay` (`afplay` on macOS; WAV only on Windows). `rookey ui` has all of it under While you talk, with a Play button for each. `ROOKEY_QUIET=1` turns the sounds off.

A small pill sits at the bottom of the screen while it listens (a level meter and the seconds so far), transcribes and types, and turns red with the reason if something failed. It's `rookey overlay`, which recordings start themselves: a layer-shell surface on Wayland (niri, Hyprland, sway, KDE), a borderless window on every Space on macOS, a layered window on Windows. It never takes a click or the focus, and it quits once rookey is idle.

- `ROOKEY_PILL=compact` drops the clock and the words, `ROOKEY_PILL=dot` shows one dot. A failure is the whole pill in every style.
- `ROOKEY_PILL_AT=x,y` moves it, in percent of the screen from the top left. `50,100`, the bottom centre, is the default.
- `ROOKEY_NO_OVERLAY=1` turns it off.

With the pill off, or on X11, a desktop notification says the same; `ROOKEY_NO_NOTIFICATIONS=1` turns those off too. GNOME has no layer shell, so it can't show the pill: set `ROOKEY_NO_OVERLAY=1` there to get the notifications. Windows has no notifications, only the sounds.

## Engines

`ROOKEY_BACKEND` picks one:

- `local`, the default: whisper.cpp on this machine, on CUDA, Metal or the CPU, whichever your build has. Needs a model.
- `elevenlabs`: ElevenLabs Scribe. Uploads the clip when you stop: about 1.5 s of waiting for 10 s of speech.
- `elevenlabs-realtime`: streams while you talk and shows the words as they come, about 0.3 s of waiting after you stop.

The ElevenLabs engines need no model, only `ELEVENLABS_API_KEY` in the [keys file](#settings).

`rookey setup` downloads the models for `local` from whisper.cpp's page on Hugging Face into the data folder:

| Model | Size | |
|---|---|---|
| Large v3 turbo | 1.5 GB | The default. Close to the largest model in accuracy, several times faster |
| Large v3 turbo, compressed | 547 MB | The same at a third of the size, a little less exact. For less memory |
| Large v3 | 2.9 GB | The largest and the slowest. Wants a GPU |
| Small | 465 MB | Quick without a GPU. More mistakes, above all outside English |
| Base | 141 MB | The quickest, and the least exact |

The settings page lists any ggml model already on disk as well: its own, and those of whisper.cpp and pywhispercpp. `ROOKEY_MODEL` takes the path of any other.

`ROOKEY_LANG` is `auto` (the default), one language like `en`, or several like `en,uk`. The local engine picks the likeliest of the ones you list each time. ElevenLabs takes one language or detects it, so with several it detects.

### What leaves your machine

- With `local`, nothing you say. ElevenLabs gets the audio, your `ROOKEY_WORDS` and any screen terms.
- Screen terms read by `ocr` stay on this machine until they go to ElevenLabs with the audio. `openai` and `anthropic` get the screenshot itself.
- The [update check](#updates) asks GitHub for the latest release. It is the only call rookey makes without being asked. Models come from Hugging Face when you pick one.
- Transcripts, settings and keys stay on disk. A saved key is never sent back to the settings page, only its last four characters.

## Settings

`rookey ui` opens the settings page. Its five sections follow a dictation: Hotkey, Engine and languages, What gets typed, While you talk, System. It is served from the rookey binary on 127.0.0.1 at a random port, fonts included, so it looks the same on every system and works offline. It opens in rookey's own window (`rookey-window`, on the system's webview) and falls back to the browser. The link carries a one-time token, and the server stops when you close the page. It can also:

- download Whisper models, and list the ones already on disk
- listen for the hotkey itself, or bind it on niri 26.04+ and Hyprland ([Hotkeys](#hotkeys))
- record a test through the settings as they are, and show what came out instead of typing it

By hand: environment variables, or `KEY=value` lines in the config file. The environment wins, so `ROOKEY_BACKEND=local rookey` overrides the file for one run. Unset, empty, `0` and `false` all mean off. When the page saves, comments and settings it doesn't know are left alone.

| | Config file | Data folder: API keys, models, history |
|---|---|---|
| Linux | `~/.config/rookey/config` | `~/.local/share/rookey/` |
| macOS | `~/Library/Application Support/rookey/config` | the same folder |
| Windows | `%APPDATA%\rookey\config` | the same folder |

```
# ~/.config/rookey/config
ROOKEY_BACKEND=elevenlabs-realtime
ROOKEY_SANITIZE=1
ROOKEY_CONTEXT=1
ROOKEY_READER=openai
ROOKEY_EDIT=Fix the punctuation, drop filler words like "ну", "типа", "короче". Keep the language.
```

API keys have a file of their own, `keys` in the data folder, in the same format and readable only by you. On Linux it is outside `~/.config` on purpose: that is what dotfile managers sync and what ends up in public repos. A hotkey-spawned `rookey toggle` doesn't see your shell's environment, so the file is where keys belong. A key an older rookey left in the config file still works, and `rookey ui` moves it over when it starts.

```
# ~/.local/share/rookey/keys
ELEVENLABS_API_KEY=sk_...
OPENAI_API_KEY=sk-...
ANTHROPIC_API_KEY=sk-ant-...
```

Every setting, in the page's order:

| Setting | |
|---|---|
| **Hotkey** | |
| `ROOKEY_HOTKEY` | the keys `rookey listen` waits for: `Super+Shift+D`, or one key like `Control_R`, `Alt_R`, `F13` |
| **Engine and languages** | |
| `ROOKEY_BACKEND` | `local` (default), `elevenlabs`, `elevenlabs-realtime` ([Engines](#engines)) |
| `ROOKEY_MODEL` | path to the ggml model, for `local` |
| `ROOKEY_LANG` | `auto` (default), one language like `en`, or several like `en,uk` |
| **What gets typed** | |
| `ROOKEY_SANITIZE=1` | drops filler words, false starts and non-speech sounds (Scribe `no_verbatim`, no extra cost). Whisper skips most of those anyway, so on `local` it only mutes non-speech tokens |
| `ROOKEY_EDIT=<instruction>` | free-form cleanup of the final transcript (Scribe `transcript_edit`, costs extra, experimental on realtime). ElevenLabs only. If the edit fails you get the transcript as it was |
| `ROOKEY_EDIT_CUSTOM` | your own instruction, kept here by `rookey ui` while one of its presets is in `ROOKEY_EDIT` |
| `ROOKEY_WORDS=<a,b,c>` | your own names and jargon, comma-separated, always passed along: Scribe `keyterms` (costs extra; realtime takes the first 50 of up to 20 characters), whisper's initial prompt on `local`. They go first, before any screen terms |
| `ROOKEY_CONTEXT=1` | reads the screen when recording starts; the terms found on it help the recognizer the same way ([Screen terms](#screen-terms)) |
| `ROOKEY_CONTEXT=<command>` | the same, with the text taken from your command's stdout |
| `ROOKEY_READER` | who reads the screenshot: `ocr` (default, tesseract on this machine), `openai`, `anthropic` |
| `ROOKEY_READER_MODEL` | the vision model, if not `gpt-6-luna` or `claude-opus-5-5` |
| `ROOKEY_SCREENSHOT=<command>` | a command that prints the image, instead of `grim` or `screencapture` |
| `ROOKEY_KEEP_CLIPBOARD=0` | macOS: leave the typed text on the clipboard instead of putting yours back |
| **While you talk** | |
| `ROOKEY_SOUNDS` | `notes` (default), `rook`, `pencil` |
| `ROOKEY_SOUND_START`, `_STOP`, `_TYPED`, `_FAILED` | a sound file of your own for that cue |
| `ROOKEY_QUIET=1` | no sounds |
| `ROOKEY_PILL` | `full` (default), `compact`, `dot` |
| `ROOKEY_PILL_AT=x,y` | where the pill sits, in percent from the top left; `50,100` by default |
| `ROOKEY_NO_OVERLAY=1` | no pill |
| `ROOKEY_NO_NOTIFICATIONS=1` | no desktop notifications |
| **System** | |
| `ROOKEY_HISTORY=0` | keep no transcripts |
| `ROOKEY_AUTOUPDATE=0` | only check for a new release and say so, don't install it ([Updates](#updates)) |
| `ROOKEY_UI_LANG` | `en` or `uk`, for the page and everything rookey says. Empty follows the system |
| `ROOKEY_UI_THEME` | `light` or `dark` for the page. Empty follows the system |
| `ROOKEY_UI_CLOSED` | the page's folded sections, kept by the page |

### Screen terms

With `ROOKEY_CONTEXT=1`, rookey takes a screenshot as recording starts and passes the names on it to the engine. `rookey -vv` shows the terms that came out of it.

The screenshot is taken with `grim` on Linux, of the focused output on niri and Hyprland and of every output elsewhere, and with `screencapture` of the main display on macOS. On Windows and X11, set `ROOKEY_SCREENSHOT` to a command that prints one.

- `ocr`: tesseract reads it on this machine, in about 2 s while you talk. Only the picked terms leave the machine. It reads English, and the `ROOKEY_LANG` languages whose tesseract packs are installed. It picks names written `like_this`, `likeThis` or `LIKE_THIS` and misses plain lowercase jargon.
- `openai`, `anthropic`: the screenshot itself goes to the provider, and a vision model lists the terms. Whatever is on the screen at that moment is in it.

ElevenLabs has no API that reads images, so it can't be a reader.

```
# a glossary of your own instead of the screen
ROOKEY_CONTEXT=cat ~/.config/rookey/glossary.txt
```

## Status for bars

`rookey status` says what rookey is doing now, and `--follow` prints a new line on every change, so a bar never polls:

```sh
$ rookey status
listening 0:04
$ rookey status --json --follow
{"state":"idle"}
{"level":0.41,"seconds":0,"state":"listening"}
{"state":"transcribing"}
{"state":"typed","waited_ms":310,"words":12}
{"state":"idle"}
```

The states are `idle`, `listening` (with `seconds` and the mic's peak `level`, 0 to 1), `transcribing`, `typed` (shown for a second, with `words` and `waited_ms`, the wait after you stopped), and `failed` (with `reason`, kept until the next recording). Any recording writes it, from a hotkey or a terminal. It's one JSON file, `$XDG_RUNTIME_DIR/rookey.status`, if you'd rather watch that.

Waybar: `--waybar` prints its format, with the state as `alt` and `class`.

```jsonc
"custom/rookey": {
  "exec": "rookey status --follow --waybar",
  "return-type": "json",
  "format": "{icon} {text}",
  "format-icons": { "idle": "", "listening": "●", "transcribing": "…", "typed": "✓", "failed": "!" },
  "on-click": "rookey toggle"
}
```
```css
#custom-rookey.listening { color: #e5584c; }
#custom-rookey.failed { color: #f0645a; }
```

Quickshell reads the same lines:

```qml
import Quickshell.Io

Process {
  command: ["rookey", "status", "--json", "--follow"]
  running: true
  stdout: SplitParser { onRead: line => root.rookey = JSON.parse(line) }
}
// root.rookey.state, .seconds, .level, .words, .reason
```

## Updates

Installed with the scripts above, rookey keeps itself up to date. When `rookey ui` or `rookey listen` starts, and once a day while `rookey listen` runs, it asks GitHub whether a new release is out. A newer one is downloaded in the background, checked against the release's `SHA256SUMS`, run once with `--version`, and put in place of the old binary. What runs keeps the old one, so the new one is used from the next start. `ROOKEY_AUTOUPDATE=0` only checks and tells you, on the settings page and with `rookey update --check`. A rookey from a package manager, Homebrew, Nix, cargo or a system folder like `/usr` or Program Files is left alone: update it where it came from.

## When something goes wrong

1. `rookey status --json`. A `failed` state carries the reason, kept until the next recording: a missing key, a missing model, no microphone access.
2. `rookey history` shows whether the words were heard and only the typing failed. The text is saved before it is typed, so it can be copied from there.
3. `rookey -vv` records in the terminal and prints every step with its time: engine, model, screen terms, requests. Stop it with Enter.
4. Try one change without saving it: `ROOKEY_BACKEND=local rookey -vv`.

On Linux, `journalctl --user -u rookey-listen -f` follows what the hotkey listener hears and does.

| What you see | Why, and what to do |
|---|---|
| It works in the terminal, but nothing comes from the hotkey | The hotkey doesn't see your shell's environment. Move the key into the keys file (`rookey ui` does it). On macOS, check Rookey's permissions on the settings page |
| "heard no words" | The wrong microphone, or it's muted. On macOS, microphone access may be off for Rookey |
| It starts and stops at once, or records twice | `rookey listen` and a desktop bind of `rookey toggle` are on the same keys. Set the hotkey in `rookey ui`, which keeps one |
| The hotkey does nothing on Linux | `rookey listen` can't read `/dev/input`. Join the `input` group (`sudo usermod -aG input $USER`) and log in again |
| Nothing typed on Linux | `wtype` is missing, or the desktop isn't Wayland. `rookey history` has the text |
| Slow on `local` | A large model on a CPU build. Pick a smaller model in `rookey setup`, or `elevenlabs-realtime` |
| Names and jargon come out wrong | Add them to `ROOKEY_WORDS`, or turn on `ROOKEY_CONTEXT=1` |

Coding agents can do this for you: `rookey skills --install` teaches them how.

## Builds

| Build | Runs on |
|---|---|
| `x86_64-linux-cuda` | an NVIDIA card with the CUDA 13 runtime installed (`libcublas.so.13`); the installer falls back to the CPU build without it |
| `x86_64-linux` | any x86_64 Linux |
| `aarch64-macos` | Apple silicon, on Metal |
| `x86_64-windows-cuda` | an NVIDIA card; the zip brings the CUDA runtime, the driver is enough |
| `x86_64-windows` | any 64-bit Windows 10 or 11 |

The installers take `ROOKEY_BUILD=cpu` (or `$env:ROOKEY_BUILD = "cpu"`) to skip the CUDA build, `ROOKEY_VERSION=v0.3.0` to pick a release, and `ROOKEY_BIN_DIR` to put it somewhere other than `~/.local/bin`.

From source, with cmake and a C/C++ compiler (whisper.cpp is built too):

```
cargo install --path .                    # CPU
cargo install --path . --features cuda    # NVIDIA (needs nvcc on PATH)
cargo install --path . --features metal   # macOS
cargo install --path window               # the settings window (Linux: needs WebKitGTK 4.1), else rookey ui opens the browser
```

### Uninstall

Turn the hotkey off first: **Stop listening** or **Unbind** on the settings page removes the login service (systemd, the Run key or the launchd agent) and the compositor bind. Then delete `rookey` and `rookey-window` from where the installer put them, the config file, and the data folder ([Settings](#settings)), which holds the models, keys, history and, on macOS, Rookey. On macOS, `~/Library/Caches/rookey` holds the settings window's app.

## Development

`just` lists the commands: `just install` builds, replaces `~/.local/bin/rookey` and restarts the hotkey listener, `just ui` does that and opens a fresh settings page, `just test` runs the tests, `just logs` follows the listener, `just site` serves the landing page on localhost. The build is CUDA on Linux and Metal on macOS; `FEATURES=` builds for the CPU. Stay on one: switching rebuilds whisper.cpp, which takes minutes.

The tests that call ElevenLabs, OpenAI or Anthropic are skipped unless asked for, with your keys and the features you build with:

```
ELEVENLABS_API_KEY=sk_... cargo test --release --features cuda -- --ignored          # the engines, on a sample clip
OPENAI_API_KEY=sk-... cargo test --release --features cuda -- --ignored screen       # the screen readers
```

- [AGENTS.md](AGENTS.md) lists the rules a change must keep, for people and coding agents alike.
- A new language is one file: [locales/README.md](locales/README.md).
- Pull request titles are [Conventional Commits](https://www.conventionalcommits.org) (`feat(ui): …`, `fix: …`). A `v*` tag builds every archive above and writes the release notes from them (`cliff.toml`).
- Security reports go through [SECURITY.md](SECURITY.md), not public issues.

## License

[Apache-2.0](LICENSE).
