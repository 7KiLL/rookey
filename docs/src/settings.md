# Settings

`rookey ui` opens the settings page. Its five sections follow a dictation: Hotkey, Engine and languages, What gets typed, While you talk, System. It is served from the rookey binary on 127.0.0.1 at a random port, fonts included, so it looks the same on every system and works offline. It opens in rookey's own window (`rookey-window`, on the system's webview) and falls back to the browser. The link carries a one-time token, and the server stops when you close the page. It can also:

- download Whisper models, and list the ones already on disk
- listen for the hotkey itself, or bind it on niri 26.04+ and Hyprland ([Hotkeys](./hotkeys.md))
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
| `ROOKEY_BACKEND` | `local` (default), `elevenlabs`, `elevenlabs-realtime` ([Engines](./engines.md)) |
| `ROOKEY_MODEL` | path to the ggml model, for `local` |
| `ROOKEY_LANG` | `auto` (default), one language like `en`, or several like `en,uk` |
| **What gets typed** | |
| `ROOKEY_SANITIZE=1` | drops filler words, false starts and non-speech sounds (Scribe `no_verbatim`, no extra cost). Whisper skips most of those anyway, so on `local` it only mutes non-speech tokens |
| `ROOKEY_EDIT=<instruction>` | free-form cleanup of the final transcript (Scribe `transcript_edit`, costs extra, experimental on realtime). ElevenLabs only. If the edit fails you get the transcript as it was |
| `ROOKEY_EDIT_CUSTOM` | your own instruction, kept here by `rookey ui` while one of its presets is in `ROOKEY_EDIT` |
| `ROOKEY_WORDS=<a,b,c>` | your own names and jargon, comma-separated, always passed along: Scribe `keyterms` (costs extra; realtime takes the first 50 of up to 20 characters), whisper's initial prompt on `local`. They go first, before any screen terms |
| `ROOKEY_CONTEXT=1` | reads the screen when recording starts; the terms found on it help the recognizer the same way ([Screen terms](./screen-terms.md)) |
| `ROOKEY_CONTEXT=<command>` | the same, with the text taken from your command's stdout |
| `ROOKEY_READER` | who reads the screenshot: `ocr` (default, tesseract on this machine), `openai`, `anthropic` |
| `ROOKEY_READER_MODEL` | the vision model, if not `gpt-6-luna` or `claude-opus-5-5` |
| `ROOKEY_SCREENSHOT=<command>` | a command that prints the image, instead of `grim` or `screencapture` |
| `ROOKEY_KEEP_CLIPBOARD=0` | macOS, GNOME, KDE: leave the typed text on the clipboard instead of putting yours back |
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
| `ROOKEY_AUTOUPDATE=0` | only check for a new release and say so, don't install it ([Updates](./install.md#updates)) |
| `ROOKEY_UI_LANG` | `en` or `uk`, for the page and everything rookey says. Empty follows the system |
| `ROOKEY_UI_THEME` | `light` or `dark` for the page. Empty follows the system |
| `ROOKEY_UI_CLOSED` | the page's folded sections, kept by the page |
