# rookey agent rules

## Build, test
- `just test`: cuda Linux, metal macOS, `FEATURES=` CPU. Never switch profile or features: whisper.cpp rebuild takes minutes.
- Window crate separate: `cargo test --release -p rookey-window`.
- `cargo fmt --all` before commit. CI checks.
- Live API tests `#[ignore]`: `ELEVENLABS_API_KEY=… cargo test --release --features cuda -- --ignored`; `OPENAI_API_KEY`, `ANTHROPIC_API_KEY` for screen readers. No keys: report untested API paths.
- No-keyboard end-to-end run: `timeout -s INT 3 ./target/release/rookey -vv` records 3 s, transcribes locally.
- Windows: type-check only, `XWIN_ACCEPT_LICENSE=1 CARGO_TARGET_DIR=<scratch> cargo xwin check --all-targets --target x86_64-pc-windows-msvc` (needs `cargo-xwin`, `ninja`). Report Windows code untested.

## Safety
- Never touch real config, data, compositor files. Copy compositor configs into `<scratch>/desk`, run `XDG_CONFIG_HOME=<scratch>/desk XDG_DATA_HOME=<scratch>/data ./target/release/rookey ui --no-open`. After unbind, `diff -r` against originals must be empty.
- XDG vars don't isolate systemd: `rookey listen`, `rookey update` restart real user service.
- Never `pkill -f`: kills own shell. `rookey ui` exits 3 s after last page closes; else kill by pid.
- Test Hyprland code from niri session: `env -u NIRI_SOCKET HYPRLAND_INSTANCE_SIGNATURE=x`. Validate copies: `niri validate -c <file>`, `Hyprland --verify-config -c <file>`.
- Env vars override `<config_dir>/rookey/config`: test settings without editing file.
- API keys only in `<data_dir>/rookey/keys` (0600), never config dir. Never send key to browser: `state()` masks.
- Page server: keep one-time token, 64 KB request cap, CSP, `changes()` whitelist.
- Compositor writes only via `desktop.rs`: inside compositor config tree, chord injection-checked, compositor-validated, byte-exact restore on failure. niri allows one `binds {}` per file: rookey owns `rookey.kdl`.
- Only `rookey-window` links webview.
- Bundle ids `io.github.7kill.rookey`, `….settings` frozen: macOS keys permissions to them.
- Never name other dictation products anywhere.

## Checklists
- New setting: `SETTINGS` + `changes()` validation arm + test (`ui.rs`); `app.js` section + `locales/` words; `docs/src/settings.md` table, page order. Common setting: also `--help` list (`cli.rs`), `src/skill.md`.
- New command: `src/skill.md` (test enforces), `docs/src/commands.md`.
- README is the pitch, about 1,000 words: new reference goes in `docs/src/` (VitePress, `just docs`), never a new README section.
- `listen.rs`, `listen_win.rs`, `listen_mac.rs` share function set: change together.

## Page (`src/ui/`)
- Assets compiled in (`include_bytes!`): rebuild, restart `rookey ui` after edit.
- arrow.js 1.0.6 vendored, docs https://arrow-js.com/llms.txt. Slot updates only as function: `${() => ui.s.x}`; plain read renders once.
- CSP: only `'self'`, no `style` attributes, no direct DOM writes.
- Strings: keys in `locales/en.json` and `locales/uk.json`, both complete (test).
- `app.css` dark tokens exist twice (`prefers-color-scheme`, `[data-theme="dark"]`): change both.
- Rare setting: section's **More** fold, never new section.
- Page preferences in config, never `localStorage`: new port each run, storage empty.
- Verify in browser: 1440x900, 390x844, light, dark, both languages.

## Git, style
- PR title: Conventional Commit (`feat(ui): …`). Squash merge; `cliff.toml` drops others from release notes.
- Deliberate shortcut: `ponytail:` comment naming limit, upgrade path.
- Non-trivial branch, parser, security path: one small test.
- ElevenLabs, OpenAI, Anthropic, niri, Hyprland: read current docs first. APIs changed 2025–2026.
