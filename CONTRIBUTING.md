# Contributing

`just` lists the commands: `just install` builds, replaces `~/.local/bin/rookey` and restarts the hotkey listener, `just ui` does that and opens a fresh settings page, `just test` runs the tests, `just logs` follows the listener, `just site` serves the landing page on localhost. The build is CUDA on Linux and Metal on macOS; `FEATURES=` builds for the CPU. Stay on one: switching rebuilds whisper.cpp, which takes minutes.

The tests that call ElevenLabs, OpenAI or Anthropic are skipped unless asked for, with your keys and the features you build with:

```
ELEVENLABS_API_KEY=sk_... cargo test --release --features cuda -- --ignored          # the engines, on a sample clip
OPENAI_API_KEY=sk-... cargo test --release --features cuda -- --ignored screen       # the screen readers
```

- [AGENTS.md](AGENTS.md) lists the rules a change must keep, for people and coding agents alike.
- A new language is one file: [locales/README.md](locales/README.md).
- Run `cargo fmt --all` before a commit: CI fails on unformatted code.
- Pull request titles are [Conventional Commits](https://www.conventionalcommits.org) (`feat(ui): …`, `fix: …`). A `v*` tag builds every archive above and writes the release notes from them (`cliff.toml`).
- Security reports go through [SECURITY.md](SECURITY.md), not public issues.

