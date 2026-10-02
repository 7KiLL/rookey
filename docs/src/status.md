# Status for bars

`rookey status` says what rookey is doing now, and `--follow` prints a new line on every change, so a bar never polls:

```sh
$ rookey status
listening 0:04
$ rookey status --json --follow
{"state":"idle"}
{"level":0.41,"seconds":0,"state":"listening"}
{"state":"transcribing"}
{"state":"typed","waited_ms":310,"words":12}
{"state":"idle"}
```

The states are `idle`, `listening` (with `seconds` and the mic's peak `level`, 0 to 1), `transcribing`, `typed` (shown for a second, with `words` and `waited_ms`, the wait after you stopped), and `failed` (with `reason`, kept until the next recording). Any recording writes it, from a hotkey or a terminal. It's one JSON file, `$XDG_RUNTIME_DIR/rookey.status`, if you'd rather watch that.

Waybar: `--waybar` prints its format, with the state as `alt` and `class`.

```jsonc
"custom/rookey": {
  "exec": "rookey status --follow --waybar",
  "return-type": "json",
  "format": "{icon} {text}",
  "format-icons": { "idle": "", "listening": "●", "transcribing": "…", "typed": "✓", "failed": "!" },
  "on-click": "rookey toggle"
}
```
```css
#custom-rookey.listening { color: #e5584c; }
#custom-rookey.failed { color: #f0645a; }
```

Quickshell reads the same lines:

```qml
import Quickshell.Io

Process {
  command: ["rookey", "status", "--json", "--follow"]
  running: true
  stdout: SplitParser { onRead: line => root.rookey = JSON.parse(line) }
}
// root.rookey.state, .seconds, .level, .words, .reason
```
