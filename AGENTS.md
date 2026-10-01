# rookey: notes for agents

Dictation CLI in Rust: record the mic, transcribe (whisper.cpp or ElevenLabs Scribe), type the text. `rookey ui` serves a settings page on localhost; `window/` is a separate workspace crate (`rookey-window`) that shows it in a webview.

## Build and test

- `just test` (cuda on Linux, metal on macOS, `FEATURES=` for CPU), `cargo test --release -p rookey-window` for the window, and `cargo fmt --all` (CI checks it). Never switch profile or features: whisper.cpp rebuilds from scratch, which takes minutes.
- Live API tests are `#[ignore]`: `ELEVENLABS_API_KEY=… cargo test --release --features cuda -- --ignored`, plus `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` for the screen readers. Without keys, say which API paths weren't exercised.
- End to end without a keyboard: `timeout -s INT 3 ./target/release/rookey -vv` records 3 s and transcribes locally.
- Windows can only be type-checked here (needs `cargo-xwin` and `ninja`): `XWIN_ACCEPT_LICENSE=1 CARGO_TARGET_DIR=<scratch> cargo xwin check --all-targets --target x86_64-pc-windows-msvc`. Say that nothing Windows-only ran.

## Rules

- **Tests never touch the real config, data or compositor files.** Copy the compositor configs into `<scratch>/desk`, then `XDG_CONFIG_HOME=<scratch>/desk XDG_DATA_HOME=<scratch>/data ./target/release/rookey ui --no-open`. After an unbind, `diff -r` against the originals must be empty.
- **XDG dirs don't isolate systemd.** `rookey listen` and `rookey update` restart the real user service; check before running them.
- **Kill by pid from a pidfile, never `pkill -f`**: it matches your own shell.
- **Settings** come from env vars, else `<config_dir>/rookey/config` (`KEY=value`). **API keys** live in `<data_dir>/rookey/keys` (0600), never under the config dir (people publish their dotfiles), and never reach the browser: `state()` sends only a mask.
- **The page server trusts nothing.** Keep the one-time token, the 64 KB request cap, the CSP and the setting whitelist in `changes()`. A new setting gets a validation arm there and a test.
- **Compositor edits go through `desktop.rs`**: only inside the compositor's own config tree, chord checked against injection, validated by the compositor (`niri validate`, `Hyprland --verify-config`), restored byte for byte on failure. niri allows one `binds {}` per file, hence the included `rookey.kdl`.
- **`listen.rs`, `listen_win.rs` and `listen_mac.rs` expose the same functions**; change them together.
- **Only `rookey-window` links a webview.** The bundle ids `io.github.7kill.rookey` and `….settings` never change: macOS keys its permissions to them.
- **A new command or setting goes in `src/skill.md`** (a test checks the commands) and in the settings list of `--help` in `cli.rs`.
- **Don't name other dictation products** in code, docs, commits or UI copy.

## The page (`src/ui/`)

- Baked in with `include_bytes!`: rebuild and restart `rookey ui` after any change.
- [arrow.js](https://arrow-js.com/llms.txt) 1.0.6, vendored because the CSP allows only `'self'` and no `style` attributes. A slot updates only when given a function (`${() => ui.s.x}`); a plain read draws once. No direct DOM writes.
- Every word shown is a key in `locales/en.json` and `locales/uk.json` (a test keeps both complete); Rust reads them with `i18n::t()`. Format rules: `locales/README.md`.
- Tokens are in `app.css` `:root`; the dark set is written twice (`prefers-color-scheme` and `[data-theme="dark"]`), change both. Commissioner for text that's read, Martian Mono for text that's typed.
- A setting few people need goes in its section's **More** fold, never a new section.
- Page preferences go in the config, not `localStorage`: each run gets a new port, so a new origin.
- Copy says what happens, what it costs, where it's saved. Errors say what to do next.
- Check changes in a real browser at 1440x900 and 390x844, light and dark, both languages.

## Style

- Mark deliberate shortcuts with a `ponytail:` comment that names the limit and the way up.
- Every non-trivial branch, parser or security path leaves one small test.
- Commit and PR titles are Conventional Commits (`feat(ui): …`): PRs are squash-merged, and `cliff.toml` drops anything else from the release notes.
- Read the current docs before writing to ElevenLabs, OpenAI, Anthropic, niri or Hyprland; several changed in 2025–2026.
