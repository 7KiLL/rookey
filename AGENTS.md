# rookey: notes for agents

Rust dictation CLI: record mic, transcribe (whisper.cpp or ElevenLabs Scribe), type text. `rookey ui` serves settings page on localhost. `window/`: separate workspace crate `rookey-window`, shows page in webview.

## Build and test

- `just test` (cuda on Linux, metal on macOS, `FEATURES=` for CPU). Window: `cargo test --release -p rookey-window`. Before commit: `cargo fmt --all` (CI checks).
- Never switch profile or features: whisper.cpp rebuilds from scratch, takes minutes.
- Live API tests are `#[ignore]`: `ELEVENLABS_API_KEY=… cargo test --release --features cuda -- --ignored`, plus `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` for screen readers. Without keys, report which API paths not exercised.
- End to end, no keyboard: `timeout -s INT 3 ./target/release/rookey -vv` records 3 s, transcribes locally.
- Windows: type-check only (needs `cargo-xwin`, `ninja`): `XWIN_ACCEPT_LICENSE=1 CARGO_TARGET_DIR=<scratch> cargo xwin check --all-targets --target x86_64-pc-windows-msvc`. Report no Windows-only code ran.

## Rules

- **Tests never touch real config, data or compositor files.** Copy compositor configs into `<scratch>/desk`, run `XDG_CONFIG_HOME=<scratch>/desk XDG_DATA_HOME=<scratch>/data ./target/release/rookey ui --no-open`. After unbind, `diff -r` against originals must be empty.
- **XDG dirs don't isolate systemd.** `rookey listen` and `rookey update` restart real user service. Check before running.
- **Never `pkill -f`**: matches own shell. Kill by pid from pidfile.
- **Settings**: env vars win over `<config_dir>/rookey/config` (`KEY=value`). **API keys**: `<data_dir>/rookey/keys` (0600), never config dir (people publish dotfiles). Keys never reach browser: `state()` sends mask only.
- **Page server trusts nothing.** Keep one-time token, 64 KB request cap, CSP, setting whitelist in `changes()`.
- **New setting**: `SETTINGS` + validation arm in `changes()` + test (`ui.rs`), page section in `app.js` + words in `locales/`, README settings table (page order). Common one also in `--help` list (`cli.rs`) and `src/skill.md`.
- **New command**: `src/skill.md` (test checks), README Use section.
- **Compositor edits only via `desktop.rs`**: write only inside compositor's own config tree, check chord against injection, validate with compositor (`niri validate`, `Hyprland --verify-config`), restore byte for byte on failure. niri allows one `binds {}` per file, so rookey includes own `rookey.kdl`.
- **`listen.rs`, `listen_win.rs`, `listen_mac.rs` expose same functions.** Change together.
- **Only `rookey-window` links webview.** Bundle ids `io.github.7kill.rookey` and `….settings` never change: macOS keys permissions to them.
- **Never name other dictation products** in code, docs, commits, UI copy.

## The page (`src/ui/`)

- Baked in with `include_bytes!`: rebuild, restart `rookey ui` after any change.
- [arrow.js](https://arrow-js.com/llms.txt) 1.0.6, vendored: CSP allows only `'self'`, no `style` attributes. Slot updates only when given function (`${() => ui.s.x}`); plain read draws once. No direct DOM writes.
- Every shown word: key in `locales/en.json` and `locales/uk.json` (test keeps both complete). Rust reads via `i18n::t()`. Format rules: `locales/README.md`.
- Tokens in `app.css` `:root`. Dark set written twice (`prefers-color-scheme`, `[data-theme="dark"]`): change both. Commissioner for read text, Martian Mono for typed text.
- Rare setting goes in its section's **More** fold, never new section.
- Page preferences go in config, not `localStorage`: each run gets new port, so new origin.
- Copy says what happens, what it costs, where it's saved. Errors say what to do next.
- Check in real browser: 1440x900 and 390x844, light and dark, both languages.

## Style

- Deliberate shortcut: `ponytail:` comment naming limit and way up.
- Every non-trivial branch, parser or security path leaves one small test.
- Commit and PR titles: Conventional Commits (`feat(ui): …`). PRs squash-merged; `cliff.toml` drops other titles from release notes.
- Read current docs before writing to ElevenLabs, OpenAI, Anthropic, niri or Hyprland APIs. Several changed 2025–2026.
