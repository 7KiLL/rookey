# Security

rookey handles your microphone, your screen (for screen terms) and API keys, so reports are welcome.

**Report a vulnerability privately** through [Report a vulnerability](https://github.com/7KiLL/rookey/security/advisories/new) on this repository, not in a public issue. Say what an attacker can do, on which system, and how to reproduce it.

Only the latest release gets fixes. `rookey update` installs it.

What counts, for example:
- an API key reaching the settings page, a log, the history or anything uploaded
- another local user or a web page reaching the `rookey ui` server
- a compositor config written outside the compositor's own folder, or not restored after a failed check
- an update installed without its SHA256SUMS line matching
