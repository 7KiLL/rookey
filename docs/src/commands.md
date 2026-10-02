# Commands

```
rookey            # record until Enter, print the transcript (pipe it: rookey | wl-copy)
rookey -v         # show the words as they arrive (-vv: every step with its time, -vvv: every audio chunk)
rookey toggle     # 1st call: start recording. 2nd call: stop, transcribe, type into the focused window
rookey listen     # hold ROOKEY_HOTKEY to talk, let go to stop (a tap keeps it going)
rookey setup      # the settings page, with what is missing first
rookey ui         # the settings page, in its own window (--browser for the browser, --no-open only prints the link)
rookey status     # what rookey is doing now, for bars (--json, --waybar, --follow)
rookey history    # the last transcripts, oldest first (--clear deletes them)
rookey update     # installs a newer release (--check only says whether there is one)
rookey skills     # a skill for coding agents: how to use rookey and find out why it failed (--install puts it in ~/.claude/skills)
rookey --help     # every command, and the settings with where they are saved on this system
```

Every transcript is kept before it is typed, so text that went into the wrong window, or wasn't typed at all, can be copied again: the last 500, as JSON lines in `history` in the [data folder](./settings.md), readable only by you and never uploaded. `rookey ui` lists them with a Copy button each. `ROOKEY_HISTORY=0` keeps none. Tests on the settings page aren't kept.
