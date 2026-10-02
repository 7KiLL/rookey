#!/bin/sh
# Install rookey: the release build for this system, into ~/.local/bin. Nothing else is
# downloaded: `rookey setup` asks for a speech model, or an ElevenLabs key instead.
#
#   curl -fsSL https://rookey.click/install | sh
#
# ROOKEY_VERSION=v0.1.0 picks a release (default: the latest), ROOKEY_BUILD=cpu skips the CUDA
# build, ROOKEY_BIN_DIR puts it somewhere else. To build from source instead, see the README.
set -eu

REPO=7KiLL/rookey
BIN_DIR=${ROOKEY_BIN_DIR:-$HOME/.local/bin}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    build=x86_64-linux
    # the CUDA build runs on the NVIDIA driver plus the CUDA 13 runtime libraries
    if [ "${ROOKEY_BUILD:-}" != cpu ] && command -v nvidia-smi >/dev/null 2>&1; then
      if ldconfig -p 2>/dev/null | grep -q 'libcublas\.so\.13'; then
        build=x86_64-linux-cuda
      else
        echo "note: NVIDIA card found, but no CUDA 13 runtime (libcublas.so.13); installing the CPU build."
        echo "      Install CUDA 13 and run this again for the GPU build."
      fi
    fi
    ;;
  Darwin-arm64) build=aarch64-macos ;;
  *)
    echo "rookey: no release build for $(uname -s) $(uname -m). Build it from source, see the README." >&2
    exit 1
    ;;
esac

if [ -n "${ROOKEY_VERSION:-}" ]; then
  url="https://github.com/$REPO/releases/download/$ROOKEY_VERSION/rookey-$build.tar.gz"
else
  url="https://github.com/$REPO/releases/latest/download/rookey-$build.tar.gz"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
echo "downloading rookey-$build"
curl -fL --progress-bar -o "$tmp/rookey.tar.gz" "$url"
tar -xzf "$tmp/rookey.tar.gz" -C "$tmp"
mkdir -p "$BIN_DIR"
# a new file then a rename: a running `rookey listen` keeps its old binary until it restarts
install -m755 "$tmp/rookey" "$BIN_DIR/rookey.new"
mv "$BIN_DIR/rookey.new" "$BIN_DIR/rookey"
echo "installed $BIN_DIR/rookey"
# the settings window, beside rookey; without it (or WebKitGTK) `rookey ui` opens the browser
if [ -f "$tmp/rookey-window" ]; then
  install -m755 "$tmp/rookey-window" "$BIN_DIR/rookey-window.new"
  mv "$BIN_DIR/rookey-window.new" "$BIN_DIR/rookey-window"
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "note: $BIN_DIR is not on your PATH yet; add it to your shell's profile." ;;
esac
echo
echo "next: rookey setup   (checks your mic and what typing needs on this desktop, then a speech model or an ElevenLabs key)"
