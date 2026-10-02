//! The commands and their help: `rookey --help`, `rookey <command> --help`, and the skill that
//! teaches an agent the same (`rookey skills`).

use std::path::PathBuf;

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};

/// How to work with rookey, for a coding agent: `rookey skills`.
pub const SKILL: &str = include_str!("skill.md");

#[derive(Parser)]
#[command(
    name = "rookey",
    version = crate::update::VERSION,
    about = "Dictation: record the mic, transcribe it, print the text or type it into the focused window.",
    long_about = "Dictation: record the mic, transcribe it, print the text or type it into the focused window.\n\n\
        With no command, rookey records until Enter or Ctrl-C and prints the transcript on stdout \
        (hints and live words go to stderr), so it pipes: `rookey | wl-copy`, `rookey > note.txt`.\n\n\
        Start with `rookey setup`: it checks the microphone, then downloads a speech model or \
        takes an ElevenLabs key, and sets the hotkey.",
    after_help = "`rookey --help` also lists the settings and where they are saved.",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Say more on stderr: -v the words as they arrive, -vv every step with its time, -vvv every audio chunk
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: Option<Cmd>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Start recording; the next call stops it, transcribes, and types the text where you are
    #[command(
        long_about = "Start recording; the next call stops it, transcribes, and types the text into the focused window.\n\n\
        Made for a hotkey: bind `rookey toggle` in your desktop, skhd, Raycast or Shortcuts. \
        One press starts, the next one stops. Every transcript is kept in `rookey history` before it is typed."
    )]
    Toggle,
    /// Hold the hotkey (ROOKEY_HOTKEY) to talk, let go to stop; a short tap keeps it recording
    #[command(
        long_about = "Hold the hotkey (ROOKEY_HOTKEY) to talk, let go to stop; a tap shorter than 0.3 s keeps it recording until the next press.\n\n\
        Runs in the foreground until stopped. `rookey ui` installs it to start at login instead: \
        a systemd user service on Linux, the Run key on Windows, a launchd agent on macOS."
    )]
    Listen,
    /// The settings page, with what is missing first (the same as `rookey ui`)
    Setup(Page),
    /// The settings page: engine, models, languages, cleanup, hotkey, API keys, history
    #[command(long_about = "The settings page: engine, models, languages, cleanup, hotkey, API keys, history.\n\n\
        Served on 127.0.0.1 at a random port behind a one-time token, in rookey's own window \
        (or the browser). It stops when the page is closed, or with Ctrl-C.")]
    Ui(Page),
    /// What rookey is doing now: idle, listening, transcribing, typed or failed
    #[command(long_about = "What rookey is doing now: idle, listening, transcribing, typed or failed.\n\n\
        --follow prints a new line on every change, so a bar never polls:\n  \
        rookey status --follow --waybar")]
    Status(Status),
    /// The pill on screen while rookey listens; recordings start it themselves
    Overlay,
    /// The last transcripts, oldest first, kept only on this computer
    History {
        /// Delete them all
        #[arg(long)]
        clear: bool,
    },
    /// Install a newer release next to this binary; it is used from the next start
    Update {
        /// Only say whether there is one
        #[arg(long)]
        check: bool,
    },
    /// Print the skill that teaches a coding agent (Claude and others) to use and fix rookey
    #[command(long_about = "Print the skill that teaches a coding agent (Claude and others) to use rookey, \
        change its settings and find out why something fails.\n\n  \
        rookey skills --install     # into ~/.claude/skills/rookey/SKILL.md\n  \
        rookey skills > SKILL.md    # anywhere else")]
    Skills {
        /// Write it to ~/.claude/skills/rookey/SKILL.md instead of printing it
        #[arg(long)]
        install: bool,
    },
    // What the page asks of a fresh process, or of Rookey (see mac.rs). Not for people.
    #[command(name = "__access", hide = true)]
    Access,
    #[command(name = "__ask", hide = true)]
    Ask {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    #[command(name = "__capture", hide = true)]
    Capture {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    // `just install`: Rookey gets the new build, and its agent restarts on it
    #[command(name = "__restart-listen", hide = true)]
    RestartListen,
}

#[derive(Args)]
pub struct Page {
    /// Only print the link, for a browser somewhere else
    #[arg(long)]
    pub no_open: bool,
    /// Open it in the browser, not in rookey's own window
    #[arg(long)]
    pub browser: bool,
}

#[derive(Args)]
pub struct Status {
    /// One JSON object per line: {"state": ..., "seconds", "level", "words", "reason"}
    #[arg(long, conflicts_with = "waybar")]
    pub json: bool,
    /// Waybar's custom module format: text, alt, class, tooltip
    #[arg(long)]
    pub waybar: bool,
    /// Keep running, and print a line each time it changes
    #[arg(short, long)]
    pub follow: bool,
}

/// The command line, with the settings and their files under `--help`. Exits on a bad line,
/// `--help` or `--version`, as clap does.
pub fn parse() -> Cli {
    let matches = Cli::command().after_long_help(settings_help()).get_matches();
    Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}

/// Where the settings are, on this system, and the ones people change.
fn settings_help() -> String {
    let shown = |p: Option<PathBuf>| p.map_or("(none on this system)".into(), |p| crate::ui::tilde(&p));
    format!(
        "\x1b[1;4mSettings:\x1b[0m
  Environment variables, or KEY=value lines in {config}
  (the environment wins, so `ROOKEY_BACKEND=local rookey` changes one run).
  API keys go in {keys}, readable only by you.
  `rookey ui` edits both.

  ROOKEY_BACKEND       local (whisper.cpp, the default), elevenlabs, elevenlabs-realtime
  ROOKEY_MODEL         the ggml model file, for local
  ROOKEY_LANG          auto (the default), en, or several like en,uk
  ROOKEY_HOTKEY        the keys `rookey listen` waits for, like Super+Shift+D or Control_R
  ROOKEY_SANITIZE=1    drop filler words, false starts and noises
  ROOKEY_EDIT          an instruction to clean up the text (ElevenLabs, costs extra)
  ROOKEY_WORDS         your names and jargon, comma-separated
  ROOKEY_CONTEXT=1     read the screen as recording starts; its terms help the recognizer
  ROOKEY_READER        who reads it: ocr (tesseract, the default), openai, anthropic
  ROOKEY_HISTORY=0     keep no transcripts
  ROOKEY_QUIET=1       no sounds; ROOKEY_SOUNDS picks notes, rook or pencil
  ROOKEY_NO_OVERLAY=1  no pill on screen; ROOKEY_PILL is full, compact or dot
  ROOKEY_AUTOUPDATE=0  only say that a release is out, don't install it

  ELEVENLABS_API_KEY, OPENAI_API_KEY, ANTHROPIC_API_KEY: for the ElevenLabs engine and the screen readers.
  Every setting: https://rookey.click/docs/settings",
        config = shown(crate::config_path()),
        keys = shown(crate::keys_path()),
    )
}

/// `rookey skills [--install]`.
pub fn skill(install: bool) -> crate::Res<()> {
    if !install {
        print!("{SKILL}");
        return Ok(());
    }
    let dir = dirs::home_dir().ok_or("no home directory")?.join(".claude").join("skills").join("rookey");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    std::fs::write(&path, SKILL)?;
    println!("wrote {}", crate::ui::tilde(&path));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_holds_together() {
        Cli::command().debug_assert();
        let cli = Cli::try_parse_from(["rookey", "-vv", "status", "--json", "-f"]).unwrap();
        assert_eq!(cli.verbose, 2);
        assert!(matches!(cli.command, Some(Cmd::Status(Status { json: true, waybar: false, follow: true }))));
        assert!(Cli::try_parse_from(["rookey", "status", "--json", "--waybar"]).is_err());
        assert!(Cli::try_parse_from(["rookey", "nonsense"]).is_err());
        // what the page hands Rookey passes through untouched, flags and all
        let cli = Cli::try_parse_from(["rookey", "__ask", "microphone", "--x"]).unwrap();
        assert!(matches!(cli.command, Some(Cmd::Ask { args }) if args == ["microphone", "--x"]));
    }

    #[test]
    fn the_skill_knows_every_command() {
        for cmd in Cli::command().get_subcommands().filter(|c| !c.is_hide_set()) {
            let name = format!("rookey {}", cmd.get_name());
            assert!(SKILL.contains(&name), "SKILL.md never mentions `{name}`");
        }
        assert!(SKILL.starts_with("---\nname: rookey\ndescription: "));
    }
}
