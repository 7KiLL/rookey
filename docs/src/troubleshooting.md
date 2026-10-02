# When something goes wrong

1. `rookey status --json`. A `failed` state carries the reason, kept until the next recording: a missing key, a missing model, no microphone access.
2. `rookey history` shows whether the words were heard and only the typing failed. The text is saved before it is typed, so it can be copied from there.
3. `rookey -vv` records in the terminal and prints every step with its time: engine, model, screen terms, requests. Stop it with Enter.
4. Try one change without saving it: `ROOKEY_BACKEND=local rookey -vv`.

On Linux, `journalctl --user -u rookey-listen -f` follows what the hotkey listener hears and does.

| What you see | Why, and what to do |
|---|---|
| It works in the terminal, but nothing comes from the hotkey | The hotkey doesn't see your shell's environment. Move the key into the keys file (`rookey ui` does it). On macOS, check Rookey's permissions on the settings page |
| "heard no words" | The wrong microphone, or it's muted. On macOS, microphone access may be off for Rookey |
| It starts and stops at once, or records twice | `rookey listen` and a desktop bind of `rookey toggle` are on the same keys. Set the hotkey in `rookey ui`, which keeps one |
| The hotkey does nothing on Linux | `rookey listen` can't read `/dev/input`. Run the command `rookey setup` shows, or join the `input` group (`sudo usermod -aG input $USER`) and log in again |
| Nothing typed on Linux | `rookey status` says why: `wtype` or `wl-clipboard` is missing, `/dev/uinput` is closed to rookey (GNOME, KDE), or the desktop isn't Wayland. `rookey history` has the text |
| Slow on `local` | A large model on a CPU build. Pick a smaller model in `rookey setup`, or `elevenlabs-realtime` |
| Names and jargon come out wrong | Add them to `ROOKEY_WORDS`, or turn on `ROOKEY_CONTEXT=1` |

Coding agents can do this for you: `rookey skills --install` teaches them how.
