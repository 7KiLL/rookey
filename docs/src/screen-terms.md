# Screen terms

With `ROOKEY_CONTEXT=1`, rookey takes a screenshot as recording starts and passes the names on it to the engine. `rookey -vv` shows the terms that came out of it.

The screenshot is taken with `grim` on Linux, of the focused output on niri and Hyprland and of every output elsewhere, and with `screencapture` of the main display on macOS. On Windows and X11, set `ROOKEY_SCREENSHOT` to a command that prints one.

- `ocr`: tesseract reads it on this machine, in about 2 s while you talk. Only the picked terms leave the machine. It reads English, and the `ROOKEY_LANG` languages whose tesseract packs are installed. It picks names written `like_this`, `likeThis` or `LIKE_THIS` and misses plain lowercase jargon.
- `openai`, `anthropic`: the screenshot itself goes to the provider, and a vision model lists the terms. Whatever is on the screen at that moment is in it.

ElevenLabs has no API that reads images, so it can't be a reader.

```
# a glossary of your own instead of the screen
ROOKEY_CONTEXT=cat ~/.config/rookey/glossary.txt
```
