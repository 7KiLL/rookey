#!/bin/sh
# Install yap: cmake, whisper model, binary (CUDA on NVIDIA Linux, Metal on macOS, else CPU).
# Usage: ./install.sh   (env YAP_MODEL_NAME to pick another ggml model)
set -eu
cd "$(dirname "$0")"

MODEL_NAME=${YAP_MODEL_NAME:-ggml-large-v3-turbo.bin}

case "$(uname -s)" in
  Darwin)
    MODEL_DIR="$HOME/Library/Application Support/yap"
    FEATURES=metal
    command -v cmake >/dev/null || brew install cmake
    ;;
  *)
    MODEL_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/yap"
    [ -x /opt/cuda/bin/nvcc ] && PATH="/opt/cuda/bin:$PATH"
    if command -v nvcc >/dev/null; then FEATURES=cuda; else FEATURES=""; fi
    command -v cmake >/dev/null || sudo pacman -S --needed cmake  # ponytail: Arch only; other distros install cmake by hand
    command -v wtype >/dev/null || echo "note: 'yap toggle' needs wtype to type text"
    ;;
esac

if [ ! -f "$MODEL_DIR/$MODEL_NAME" ]; then
  mkdir -p "$MODEL_DIR"
  curl -fL -o "$MODEL_DIR/$MODEL_NAME.part" \
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$MODEL_NAME"
  mv "$MODEL_DIR/$MODEL_NAME.part" "$MODEL_DIR/$MODEL_NAME"  # no half-downloaded model on Ctrl-C
fi

echo "building with features: ${FEATURES:-cpu}"
cargo install --path . ${FEATURES:+--features "$FEATURES"}

echo "done. try: yap   (talk, then Ctrl-C)"
[ "$MODEL_NAME" = ggml-large-v3-turbo.bin ] || echo "set YAP_MODEL=\"$MODEL_DIR/$MODEL_NAME\""
