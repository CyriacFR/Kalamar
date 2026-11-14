//! The launcher's own command line, for a desktop shortcut that starts the game through it:
//!
//! ```text
//! Kalamar.exe --play full|training|roam [--minimized]
//! ```
//!
//! `--play` names an offline mode: `full` (the full game, from its title screen), `training` (the shooting range) or
//! `roam` (free roam in the hub); the page launches it exactly as its button would, with the profile and options of the
//! launcher, as soon as the install status is known (an incomplete profile or a missing install is reported on screen as
//! usual). `--minimized` opens the launcher in the taskbar instead of in front of the game. Anything else is ignored.

use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Cli {
    /// The mode id the Play screen uses: `original`, `range` or `hub`.
    pub play: Option<&'static str>,
    pub minimized: bool,
    /// Arguments that were understood badly (an unknown mode, a missing value), shown to the person once.
    pub invalid: Vec<String>,
}

/// Mode id for a `--play` value (also accepts the ids themselves).
pub fn mode_for(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "full" | "original" => Some("original"),
        "training" | "range" => Some("range"),
        "roam" | "hub" => Some("hub"),
        _ => None,
    }
}

/// Parses the arguments after the program name.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Cli {
    let mut cli = Cli::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (a.clone(), None),
        };
        match name.as_str() {
            "--minimized" => cli.minimized = true,
            "--play" => match inline.or_else(|| it.next()) {
                Some(v) => match mode_for(&v) {
                    Some(m) => cli.play = Some(m),
                    None => cli.invalid.push(format!("--play {}", v.chars().take(32).collect::<String>())),
                },
                None => cli.invalid.push("--play".into()),
            },
            _ => {}
        }
    }
    cli
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Cli {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn modes_and_minimized() {
        assert_eq!(p(&[]), Cli::default());
        assert_eq!(p(&["--play", "full"]).play, Some("original"));
        assert_eq!(p(&["--play", "training"]).play, Some("range"));
        assert_eq!(p(&["--play=roam", "--minimized"]), Cli { play: Some("hub"), minimized: true, invalid: vec![] });
        assert_eq!(p(&["--minimized"]).play, None);
        assert_eq!(p(&["--PLAY", "FULL"]).play, None); // option names are exact, values are not case sensitive
        assert_eq!(p(&["--play", "Full"]).play, Some("original"));
    }

    #[test]
    fn bad_values_are_reported_not_acted_on() {
        let c = p(&["--play", "story", "--minimized"]);
        assert_eq!((c.play, c.minimized, c.invalid.as_slice()), (None, true, &["--play story".to_string()][..]));
        assert_eq!(p(&["--play"]).invalid, vec!["--play"]);
        // LAN is never started from a shortcut; other arguments are ignored
        assert_eq!(p(&["--play", "lan"]).play, None);
        assert_eq!(p(&["--whatever", "x"]), Cli::default());
    }
}
