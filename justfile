# rookey dev commands: `just` lists them.
# FEATURES picks the whisper.cpp backend: cuda (Linux + NVIDIA, the default there), metal (the default
# on macOS), or empty for the CPU.

features := env("FEATURES", if os() == "macos" { "metal" } else { "cuda" })
cargo_features := if features == "" { "" } else { "--features " + features }

default:
    @just --list

# build the release binaries: rookey and its settings window
build:
    cargo build --release {{cargo_features}}
    cargo build --release -p rookey-window

# build, replace ~/.local/bin/rookey and its window, restart the hotkey listener if it runs
install: build sign
    for f in rookey rookey-window; do install -m755 target/release/$f ~/.local/bin/$f.new && mv ~/.local/bin/$f.new ~/.local/bin/$f; done
    -if [ "$(uname)" = Darwin ]; then ~/.local/bin/rookey __restart-listen; fi
    -if command -v systemctl >/dev/null; then systemctl --user try-restart rookey-listen; fi

# macOS: sign with ROOKEY_SIGN_P12 (and ROOKEY_SIGN_PASSWORD) through rcodesign, if set. macOS
# keeps Rookey's permissions for a certificate across builds; an unsigned build asks again.
sign:
    #!/bin/sh
    [ "$(uname)" = Darwin ] && [ -n "${ROOKEY_SIGN_P12:-}" ] || exit 0
    for pair in rookey:io.github.7kill.rookey rookey-window:io.github.7kill.rookey.settings; do
      rcodesign sign --p12-file "$ROOKEY_SIGN_P12" --p12-password "${ROOKEY_SIGN_PASSWORD:-}" \
        --binary-identifier "${pair#*:}" "target/release/${pair%%:*}" "target/release/${pair%%:*}" >/dev/null
    done
    echo "signed with $ROOKEY_SIGN_P12"

# install, close a running settings page, open a fresh one
ui: install
    -for p in $(pgrep -x rookey); do ps -o args= -p $p | grep -q ' ui' && kill $p; done
    rookey ui

test:
    cargo test --release {{cargo_features}}

# follow what the hotkey listener hears and does
logs:
    journalctl --user -u rookey-listen -f

# the landing page as it deploys, on http://localhost:8089 (Ctrl-C stops it)
site port="8089":
    docker build -q -f site/Dockerfile -t rookey-site .
    docker run --rm -p {{port}}:80 rookey-site
