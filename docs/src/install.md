# Install

```sh
curl -fsSL https://rookey.click/install | sh      # Linux, macOS
```
```powershell
irm https://rookey.click/install.ps1 | iex        # Windows, in PowerShell
```

Then, once:

```sh
rookey setup     # checks the mic, downloads a speech model (or takes an ElevenLabs key instead), sets the hotkey
```

The installers put the release build for your system on your PATH (`~/.local/bin`, or `%LOCALAPPDATA%\rookey` on Windows) and nothing else. The speech model (1.5 GB for the default) is downloaded only when you pick it in `rookey setup`, and not at all if you use ElevenLabs. Other builds, and building from source: [Builds](#builds).

What else each system needs:

| System | Needs |
|---|---|
| Linux | A Wayland desktop. niri, Hyprland, sway and COSMIC type with `wtype`; GNOME and KDE paste, with `wl-clipboard` and access to `/dev/uinput`. Hold to talk reads the keyboards. One udev rule allows both for the session at the computer, without logging out: `rookey setup` shows it. The `input` group works too |
| macOS | Apple silicon. The settings page asks for the microphone, Accessibility (the hotkey and the typing), Automation of System Events (the paste) and, for screen terms, Screen Recording. macOS files them under *Rookey*, the small app rookey runs as, not under your terminal or your hotkey app |
| Windows | 10 or 11, nothing else. The CUDA build brings its own runtime |

For [screen terms](./screen-terms.md), `tesseract` on any system, and `grim` on Linux.

## Builds

| Build | Runs on |
|---|---|
| `x86_64-linux-cuda` | an NVIDIA card with the CUDA 13 runtime installed (`libcublas.so.13`); the installer falls back to the CPU build without it |
| `x86_64-linux` | any x86_64 Linux |
| `aarch64-macos` | Apple silicon, on Metal |
| `x86_64-windows-cuda` | an NVIDIA card; the zip brings the CUDA runtime, the driver is enough |
| `x86_64-windows` | any 64-bit Windows 10 or 11 |

The installers take `ROOKEY_BUILD=cpu` (or `$env:ROOKEY_BUILD = "cpu"`) to skip the CUDA build, `ROOKEY_VERSION=v0.3.0` to pick a release, and `ROOKEY_BIN_DIR` to put it somewhere other than `~/.local/bin`.

From source, with cmake and a C/C++ compiler (whisper.cpp is built too):

```
cargo install --path .                    # CPU
cargo install --path . --features cuda    # NVIDIA (needs nvcc on PATH)
cargo install --path . --features metal   # macOS
cargo install --path window               # the settings window (Linux: needs WebKitGTK 4.1), else rookey ui opens the browser
```

## Uninstall

Turn the hotkey off first: **Stop listening** or **Unbind** on the settings page removes the login service (systemd, the Run key or the launchd agent) and the compositor bind. Then delete `rookey` and `rookey-window` from where the installer put them, the config file, and the data folder ([Settings](./settings.md)), which holds the models, keys, history and, on macOS, Rookey. On macOS, `~/Library/Caches/rookey` holds the settings window's app. On Linux, if you ran the access command from `rookey setup`, `sudo rm /etc/udev/rules.d/70-rookey.rules` takes it back.

## Updates

Installed with the scripts above, rookey keeps itself up to date. When `rookey ui` or `rookey listen` starts, and once a day while `rookey listen` runs, it asks GitHub whether a new release is out. A newer one is downloaded in the background, checked against the release's `SHA256SUMS`, run once with `--version`, and put in place of the old binary. What runs keeps the old one, so the new one is used from the next start. `ROOKEY_AUTOUPDATE=0` only checks and tells you, on the settings page and with `rookey update --check`. A rookey from a package manager, Homebrew, Nix, cargo or a system folder like `/usr` or Program Files is left alone: update it where it came from.
