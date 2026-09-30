# rookey dev commands: `just` lists them.
# FEATURES picks the whisper.cpp backend: cuda (Linux + NVIDIA, the default here), metal (macOS), or empty.

features := env("FEATURES", "cuda")
cargo_features := if features == "" { "" } else { "--features " + features }

default:
    @just --list

# build the release binary
build:
    cargo build --release {{cargo_features}}

# build, replace ~/.local/bin/rookey, restart the hotkey listener if it runs
install: build
    install -m755 target/release/rookey ~/.local/bin/rookey
    -systemctl --user try-restart rookey-listen

# install, close a running settings page, open a fresh one
ui: install
    -for p in $(pgrep -x rookey); do tr '\0' ' ' < /proc/$p/cmdline | grep -q ' ui' && kill $p; done
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
