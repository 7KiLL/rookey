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
install: build
    for f in rookey rookey-window; do install -m755 target/release/$f ~/.local/bin/$f.new && mv ~/.local/bin/$f.new ~/.local/bin/$f; done
    -if command -v systemctl >/dev/null; then systemctl --user try-restart rookey-listen; fi

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
