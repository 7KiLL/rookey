# rookey: notes for agents

rookey is a dictation CLI in Rust: record the mic, transcribe (local whisper.cpp or ElevenLabs Scribe), print or type the text. `rookey ui` serves a settings page on localhost.

## Build and test

```sh
PATH="/opt/cuda/bin:$PATH" cargo test --release --features cuda   # NVIDIA Linux
cargo test --release --features metal                              # macOS
cargo test --release                                               # CPU
```

- Stay on one profile and feature set. Switching (debug, or no `cuda`) rebuilds whisper.cpp from scratch, which takes minutes.
- Live API tests are `#[ignore]`: `ELEVENLABS_API_KEY=... cargo test --release -- --ignored`, and `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` for the screen readers. Without keys, say plainly which API paths were not exercised.
- A quick end-to-end run without a keyboard: `timeout -s INT 3 ./target/release/rookey -vv` records 3 s and transcribes locally.

## Layout

| File | What it does |
|---|---|
| `src/main.rs` | CLI, settings (`setting()`), recording, backends (whisper, Scribe batch, Scribe realtime over WebSocket), typing, the move from the old `yap` dirs |
| `src/reader.rs` | Screen terms: screenshot, then local OCR (tesseract) or a vision model (OpenAI, Claude) |
| `src/models.rs` | Whisper model catalog, finding installed models, downloads |
| `src/desktop.rs` | Hotkeys: detects niri or Hyprland, edits their config safely, validates with the compositor itself |
| `src/ui.rs` | The `rookey ui` HTTP server: token, routes, state for the page, input checks, setup checks |
| `src/ui/` | The page (`index.html`, `app.js`, `app.css`), fonts and icon, baked in with `include_bytes!` |

Settings: environment variables win over `<config_dir>/rookey/config` (`KEY=value` lines). API keys live in `<data_dir>/rookey/keys`, mode 0600, never in the config dir: people sync `~/.config` with dotfile managers and publish it.

## Rules that are easy to break

- **Never touch the real config, data or compositor files in a test.** Run the UI on copies: `XDG_CONFIG_HOME=<scratch>/desk XDG_DATA_HOME=<scratch>/data ./target/release/rookey ui --no-open`, with the compositor configs copied into `<scratch>/desk`. After an unbind, `diff -r` against the originals must come out empty.
- **Keys never reach the browser.** `state()` sends only a mask (`sk_••••••••a1b2`). Keep it that way.
- **The page server trusts nothing.** Keep the one-time token check, the setting whitelist in `changes()`, the 64 KB request cap and the CSP. Any new setting gets a validation arm in `changes()` and a test.
- **Compositor edits go through `desktop.rs`.** Only files inside the compositor's own config tree are written. They are validated by the compositor (`niri validate`, `Hyprland --verify-config`) and restored byte for byte on failure. niri allows one `binds {}` per file, so rookey writes its own `rookey.kdl` and includes it. The chord is checked against injection before it is written.
- **UI assets are compiled in.** Rebuild and restart `rookey ui` after any HTML, CSS or JS change.
- **Don't kill processes with `pkill -f <text>`.** It matches the shell running the command. Keep a pidfile and kill by pid.
- **Don't name other dictation products** in code, docs, commits or UI copy.

## The page

- The design is a steno pad. Tokens are in `app.css` `:root`, with a dark set under `prefers-color-scheme`. Commissioner is used for anything spoken or read, Martian Mono for anything typed. The left column holds settings, the right column a live example sentence, split by a red rule.
- There's a simple view anyone can set up (setup check, engine, languages, cleanup, hotkey) and an **Advanced** fold for enthusiasts. New knobs go into Advanced unless most people need them.
- Copy is plain and specific: what happens, what it costs, where things are saved. Errors say what to do next.
- Check changes in a real browser at 1440x900 and 390x844, in light and in dark.

## Style

- The code comments `ponytail:` on deliberate shortcuts. Each one names the limit and the way up. Keep adding them when you cut a corner on purpose.
- Every non-trivial branch, parser or security path leaves one small test behind.
- Before writing to a changing API (ElevenLabs, OpenAI, Anthropic, niri, Hyprland), read its current docs. Several of them changed in 2025–2026.
