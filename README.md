# yap

Local dictation. Records the mic, transcribes with whisper.cpp, prints or types the text.

```
yap            # record until Enter, print transcript (pipe it: yap | wl-copy)
yap -v         # show transcript text as it arrives (-vv: steps + timings, -vvv: every audio chunk)
yap toggle     # 1st call: start recording. 2nd call: stop, transcribe, type into focused window
```

Env: `YAP_MODEL` (path to ggml model), `YAP_LANG` (default `auto`, or `en`, `de`, `ru`, `uk`...).

Cloud backends (ElevenLabs Scribe, no local model needed), `YAP_BACKEND=`:
- `elevenlabs`: uploads the clip when you stop (~1.5 s wait for 10 s of speech)
- `elevenlabs-realtime`: streams while you talk, shows live partials, ~0.3 s wait after stop

```
YAP_BACKEND=elevenlabs-realtime ELEVENLABS_API_KEY=sk_... yap
ELEVENLABS_API_KEY=sk_... cargo test -- --ignored   # live API tests on a sample clip
```

## Setup

`./install.sh` does all of the below (picks cuda/metal/cpu). Manual steps:

Needs cmake + a C/C++ compiler (whisper.cpp is built from source).

```
# model (~1.6 GB). Default path: <data_dir>/yap/ggml-large-v3-turbo.bin
mkdir -p ~/.local/share/yap
curl -L -o ~/.local/share/yap/ggml-large-v3-turbo.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin

cargo install --path .                    # CPU
cargo install --path . --features cuda    # Linux + NVIDIA (needs nvcc on PATH)
cargo install --path . --features metal   # macOS
```

macOS model path: `~/Library/Application Support/yap/`.

## Hotkeys

niri (`~/.config/niri/config.kdl`), needs `wtype`:
```kdl
Mod+Shift+D { spawn "yap" "toggle"; }
```

macOS: bind `yap toggle` with skhd/Raycast/Shortcuts. The calling app needs Accessibility permission (it pastes via Cmd+V).
