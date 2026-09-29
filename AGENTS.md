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
- Windows can't be run here, only type-checked: `cargo install cargo-xwin` and `ninja` on PATH, then `XWIN_ACCEPT_LICENSE=1 CARGO_TARGET_DIR=<scratch> cargo xwin check --all-targets --target x86_64-pc-windows-msvc`. Say plainly that nothing Windows-only was run; the release workflow builds it for real.

## Layout

| File | What it does |
|---|---|
| `src/main.rs` | CLI, settings (`setting()`), recording, backends (whisper, Scribe batch, Scribe realtime over WebSocket), typing, the move from the old `yap` dirs |
| `src/reader.rs` | Screen terms: screenshot, then local OCR (tesseract) or a vision model (OpenAI, Claude) |
| `src/models.rs` | Whisper model catalog, finding installed models, downloads |
| `src/desktop.rs` | Hotkeys: detects niri or Hyprland, edits their config safely, validates with the compositor itself |
| `src/hold.rs` | Hold to talk, tap to keep talking: the key logic both listeners share |
| `src/listen.rs`, `src/listen_win.rs` | `rookey listen`: evdev and a systemd user service on Linux, the key state and the Run key on Windows. Same functions in both |
| `src/win.rs` | The Windows calls: typing (SendInput), a key's state, whether a pid runs |
| `install.sh`, `install.ps1` | Install the release build and nothing else; the model is picked in `rookey setup` |
| `.github/workflows/release.yml`, `cliff.toml` | A `v*` tag builds every archive and writes the notes from Conventional Commits |
| `src/ui.rs` | The `rookey ui` HTTP server: token, routes, state for the page, input checks, setup checks |
| `src/ui/` | The page: `app.js` (arrow.js templates), `i18n.js` (every word, per language), `app.css`, the vendored `arrow.js`, fonts and icon, baked in with `include_bytes!` |

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

- The design is a steno pad. Tokens are in `app.css` `:root`, with a dark set under `prefers-color-scheme` and again under `[data-theme="dark"]` for the page's own switch (`ROOKEY_UI_THEME`, empty follows the system). Commissioner is used for anything spoken or read, Martian Mono for anything typed. The left column holds settings, the right column a live example sentence, split by a red rule.
- There's a simple view anyone can set up (setup check, engine, languages, cleanup, hotkey) and an **Advanced** fold for enthusiasts. New knobs go into Advanced unless most people need them.
- Copy is plain and specific: what happens, what it costs, where things are saved. Errors say what to do next.
- The page is [arrow.js](https://arrow-js.com/llms.txt) 1.0.6, vendored as one file (`npm pack @arrow-js/core`, `bun build dist/index.mjs --minify --format esm`) because the CSP allows only `'self'`. A slot updates only when it is given a function (`${() => ui.s.x}`); a static read inside a template is drawn once. No direct DOM writes: that is also why the download bar is a native `<progress>` (the CSP blocks `style` attributes).
- Every word the page shows is a key in `i18n.js`, English and Ukrainian (`ROOKEY_UI_LANG`, empty follows the browser). A test fails if a language misses a key. Words the server writes itself (errors, blocked reasons) stay English. Changing the language reloads the page.
- Page preferences go in the config, not `localStorage`: every `rookey ui` run gets a new port, so a new origin with empty storage.
- Check changes in a real browser at 1440x900 and 390x844, in light and in dark, in both languages.

## Style

- The code comments `ponytail:` on deliberate shortcuts. Each one names the limit and the way up. Keep adding them when you cut a corner on purpose.
- Every non-trivial branch, parser or security path leaves one small test behind.
- Before writing to a changing API (ElevenLabs, OpenAI, Anthropic, niri, Hyprland), read its current docs. Several of them changed in 2025–2026.
