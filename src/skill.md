---
name: rookey
description: Use rookey, the dictation CLI (record the mic, transcribe locally with whisper.cpp or with ElevenLabs, print or type the text), and help someone set it up, change its settings, bind its hotkey or find out why a recording failed. Use when the user mentions rookey, dictation, speech to text on their desktop, or a transcript that came out wrong or not at all.
---

# rookey

rookey records the microphone, turns speech into text, and prints the text or types it into the focused window. It runs on Linux, macOS and Windows. `rookey --help` and `rookey <command> --help` are the reference; this page is how to use them well.

## Commands

| Command | What it does | Safe for you to run? |
|---|---|---|
| `rookey` | Records until Enter or Ctrl-C, prints the transcript on stdout. Hints go to stderr, so `rookey \| wl-copy` and `rookey > note.txt` work | Only when the user is ready to speak: it opens the mic |
| `rookey toggle` | First call starts recording, the second stops it and types the text into the focused window. Made for a hotkey | No: the text is typed into whatever has focus, which may be your terminal |
| `rookey listen` | Hold the hotkey (`ROOKEY_HOTKEY`) to talk, let go to stop; a tap keeps it recording. Runs in the foreground | No: `rookey ui` installs it as a login service; ask first |
| `rookey setup` | The settings page, with what is missing first | Yes, it opens a window; the user does the rest |
| `rookey ui` | The settings page: engine, models, languages, cleanup, hotkey, keys, history. `--no-open` prints the link only, `--browser` uses the browser | Yes |
| `rookey status` | What rookey is doing now: `idle`, `listening`, `transcribing`, `typed`, `failed` (with the reason). `--json`, `--waybar`, `--follow` | Yes, read-only |
| `rookey history` | The last transcripts, oldest first, with their time in UTC. `--clear` deletes them | Reading, yes. `--clear` only when asked |
| `rookey overlay` | The pill on screen during a recording. Recordings start it themselves | Rarely needed |
| `rookey update` | Installs a newer release next to the binary. `--check` only says whether there is one | `--check` yes; installing only when asked |
| `rookey skills` | Prints this page. `--install` writes it to `~/.claude/skills/rookey/SKILL.md` | Yes |

`-v`, `-vv`, `-vvv` go before or after any command and say more on stderr: the words as they arrive, then every step with its time, then every audio chunk.

## Settings

Settings are environment variables, or `KEY=value` lines in the config file. The environment wins, so `ROOKEY_BACKEND=local rookey -vv` tries one change for one run without saving it. `rookey --help` prints the file's path on this system:

- Linux: `~/.config/rookey/config`
- macOS: `~/Library/Application Support/rookey/config`
- Windows: `%APPDATA%\rookey\config`

API keys (`ELEVENLABS_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`) live in a separate file, `keys`, in the data folder (`~/.local/share/rookey/keys` on Linux, the same folder as the config on macOS and Windows), readable only by the user. A `rookey toggle` started by a hotkey doesn't see the shell's environment, so a key exported in `.zshrc` works in a terminal and not from the hotkey. The keys file works everywhere.

When you change settings:
- Prefer `rookey ui` when the user can click. It checks every value.
- By hand, edit only the lines you mean to change and keep the comments. Unset, empty, `0` and `false` all mean off.
- **Never print, cat or copy the keys file**, and never put a key in the config file: people sync and publish their config folder. To check that a key is set, `grep -c '^ELEVENLABS_API_KEY=' <keys file>` is enough.

The settings people change most:

| Setting | Values |
|---|---|
| `ROOKEY_BACKEND` | `local` (whisper.cpp, the default, needs a model), `elevenlabs` (uploads when you stop), `elevenlabs-realtime` (streams while you talk) |
| `ROOKEY_MODEL` | path to a ggml model, for `local`. `rookey setup` downloads one |
| `ROOKEY_LANG` | `auto`, one code like `en`, or several like `en,uk` |
| `ROOKEY_HOTKEY` | for `rookey listen`: `Super+Shift+D`, or one key like `Control_R`, `Alt_R`, `F13` |
| `ROOKEY_SANITIZE=1` | drops filler words, false starts, noises |
| `ROOKEY_EDIT` | an instruction for cleaning up the text. ElevenLabs only, costs extra |
| `ROOKEY_WORDS` | names and jargon, comma-separated, to help recognition |
| `ROOKEY_CONTEXT=1` | reads the screen as recording starts and passes its terms along. `ROOKEY_READER` is `ocr` (tesseract, local: English and the `ROOKEY_LANG` languages whose tesseract packs are installed), `openai` or `anthropic` (the screenshot is sent to them) |
| `ROOKEY_HISTORY=0` | keeps no transcripts |
| `ROOKEY_QUIET=1`, `ROOKEY_NO_OVERLAY=1`, `ROOKEY_NO_NOTIFICATIONS=1` | no sounds, no pill, no notifications |
| `ROOKEY_AUTOUPDATE=0` | only says that a release is out |

The README in the rookey repository lists every setting.

## When something goes wrong

1. `rookey status --json`. A `failed` state carries the reason, kept until the next recording. It's usually enough: a missing key, a missing model, no microphone access.
2. `rookey history` shows whether the words were heard and only the typing failed. The text is saved before it is typed, so nothing is lost: it can be copied from there.
3. Reproduce with the user speaking: `rookey -vv` records in the terminal and prints every step with its time (backend, model, screen terms, request times). Stop it with Enter. `timeout -s INT 5 rookey -vv` records 5 seconds unattended.
4. Try one change without saving it: `ROOKEY_BACKEND=local rookey -vv`.

Common causes:

- **Nothing typed from the hotkey, but it works in the terminal.** The hotkey doesn't see the shell's environment: move the key into the keys file (`rookey ui` does it). On macOS, check the permissions on the settings page (microphone, Accessibility, Automation of System Events). They belong to *Rookey*, the small app rookey runs as, not to the terminal.
- **Nothing typed on Linux.** `rookey status --json` says why: `wtype` is missing, there's no Wayland session, or the desktop is GNOME or KDE, where rookey can't type yet. The text is in `rookey history` either way.
- **"heard no words".** The wrong microphone, or it's muted. On macOS, microphone access may be off for Rookey.
- **Two recordings at once, or it starts and stops immediately.** Both `rookey listen` and a desktop bind of `rookey toggle` are on the same keys. `rookey ui` keeps only one.
- **Slow on `local`.** A large model on a CPU build. Suggest a smaller model in `rookey setup`, or `elevenlabs-realtime`.
- **Wrong words for names and jargon.** Add them to `ROOKEY_WORDS`, or turn on `ROOKEY_CONTEXT=1`.

## Hotkeys

- `rookey listen`, which `rookey ui` installs as a login service (systemd on Linux, the Run key on Windows, launchd on macOS). Hold to talk.
- Any desktop: bind `rookey toggle` in the compositor (niri, Hyprland: `rookey ui` writes and validates the bind), or in skhd, Raycast or Shortcuts on macOS.

Don't edit compositor configs by hand for rookey when `rookey ui` can do it: it validates the change with the compositor and undoes it if it fails.
