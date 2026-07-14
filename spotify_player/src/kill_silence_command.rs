use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillSilenceCommand {
    Song,
    Search(String),
    SpotifyDevice,
    Queue,
    Play,
    Stop,
    Replay,
    Next,
    Previous,
    Volume(u8),
    Like,
    WithAgents,
    Home,
    Player,
    Help,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KillSilenceCommandSpec {
    pub completion: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
}

pub const KILL_SILENCE_COMMANDS: &[KillSilenceCommandSpec] = &[
    KillSilenceCommandSpec {
        completion: "/song",
        usage: "/song",
        description: "saved tracks and playlists",
    },
    KillSilenceCommandSpec {
        completion: "/search ",
        usage: "/search <query>",
        description: "search Spotify",
    },
    KillSilenceCommandSpec {
        completion: "/spotify device",
        usage: "/spotify device",
        description: "choose a Connect device",
    },
    KillSilenceCommandSpec {
        completion: "/queue",
        usage: "/queue",
        description: "open the Spotify queue",
    },
    KillSilenceCommandSpec {
        completion: "/play",
        usage: "/play",
        description: "resume playback",
    },
    KillSilenceCommandSpec {
        completion: "/stop",
        usage: "/stop",
        description: "pause playback",
    },
    KillSilenceCommandSpec {
        completion: "/replay",
        usage: "/replay",
        description: "restart the current track",
    },
    KillSilenceCommandSpec {
        completion: "/next",
        usage: "/next",
        description: "skip to the next track",
    },
    KillSilenceCommandSpec {
        completion: "/prev",
        usage: "/prev",
        description: "return to the previous track",
    },
    KillSilenceCommandSpec {
        completion: "/volume ",
        usage: "/volume <1..10>",
        description: "set Spotify volume",
    },
    KillSilenceCommandSpec {
        completion: "/like",
        usage: "/like",
        description: "save the current track",
    },
    KillSilenceCommandSpec {
        completion: "/with-agents",
        usage: "/with-agents",
        description: "watch an external Claude session",
    },
    KillSilenceCommandSpec {
        completion: "/home",
        usage: "/home",
        description: "show the KILL//SILENCE title",
    },
    KillSilenceCommandSpec {
        completion: "/player",
        usage: "/player",
        description: "show the playing track",
    },
    KillSilenceCommandSpec {
        completion: "/help",
        usage: "/help",
        description: "open the command archive",
    },
    KillSilenceCommandSpec {
        completion: "/quit",
        usage: "/quit",
        description: "close KILL//SILENCE",
    },
];

#[must_use]
pub fn command_suggestions(input: &str) -> Vec<KillSilenceCommandSpec> {
    let query = input.trim_start().to_ascii_lowercase();
    if !query.starts_with('/') {
        return Vec::new();
    }
    KILL_SILENCE_COMMANDS
        .iter()
        .copied()
        .filter(|spec| spec.completion.starts_with(&query) || spec.usage.starts_with(&query))
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum KillSilenceCommandError {
    Empty,
    MissingSearchQuery,
    InvalidVolume,
    InvalidSpotifyCommand,
    Unknown(String),
}

impl std::fmt::Display for KillSilenceCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("TYPE A COMMAND — /HELP SHOWS THE ARCHIVE"),
            Self::MissingSearchQuery => {
                formatter.write_str("SEARCH NEEDS A QUERY — /SEARCH <WORDS>")
            }
            Self::InvalidVolume => formatter.write_str("VOLUME MUST BE A NUMBER FROM 1 TO 10"),
            Self::InvalidSpotifyCommand => {
                formatter.write_str("SPOTIFY COMMAND NOT RECOGNIZED — TRY /SPOTIFY DEVICE")
            }
            Self::Unknown(name) => write!(formatter, "UNKNOWN COMMAND: {name} — TRY /HELP"),
        }
    }
}

impl std::error::Error for KillSilenceCommandError {}

impl FromStr for KillSilenceCommand {
    type Err = KillSilenceCommandError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.trim().trim_start_matches('/').trim();
        if input.is_empty() {
            return Err(KillSilenceCommandError::Empty);
        }

        let mut parts = input.split_whitespace();
        let name = parts
            .next()
            .ok_or(KillSilenceCommandError::Empty)?
            .to_ascii_lowercase();
        match name.as_str() {
            "song" | "songs" | "library" => Ok(Self::Song),
            "search" => {
                let query = parts.collect::<Vec<_>>().join(" ");
                (!query.is_empty())
                    .then_some(Self::Search(query))
                    .ok_or(KillSilenceCommandError::MissingSearchQuery)
            }
            "spotify" => match parts.next().map(str::to_ascii_lowercase).as_deref() {
                Some("device" | "devices") => Ok(Self::SpotifyDevice),
                _ => Err(KillSilenceCommandError::InvalidSpotifyCommand),
            },
            "queue" => Ok(Self::Queue),
            "play" => Ok(Self::Play),
            "stop" | "pause" => Ok(Self::Stop),
            "replay" | "restart" => Ok(Self::Replay),
            "next" => Ok(Self::Next),
            "prev" | "previous" => Ok(Self::Previous),
            "volume" | "vol" => parts
                .next()
                .and_then(|value| value.parse::<u8>().ok())
                .filter(|value| (1..=10).contains(value))
                .map(Self::Volume)
                .ok_or(KillSilenceCommandError::InvalidVolume),
            "like" | "save" => Ok(Self::Like),
            "with-agents" | "agents" => Ok(Self::WithAgents),
            "home" | "main" | "title" => Ok(Self::Home),
            "player" | "now" | "now-playing" => Ok(Self::Player),
            "help" | "?" => Ok(Self::Help),
            "quit" | "exit" => Ok(Self::Quit),
            _ => Err(KillSilenceCommandError::Unknown(name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_player_and_spotify_commands() {
        assert_eq!("/play".parse(), Ok(KillSilenceCommand::Play));
        assert_eq!("stop".parse(), Ok(KillSilenceCommand::Stop));
        assert_eq!(
            "/search last night on earth".parse(),
            Ok(KillSilenceCommand::Search("last night on earth".into()))
        );
        assert_eq!(
            "/spotify device".parse(),
            Ok(KillSilenceCommand::SpotifyDevice)
        );
    }

    #[test]
    fn volume_uses_the_original_one_to_ten_scale() {
        assert_eq!("/volume 7".parse(), Ok(KillSilenceCommand::Volume(7)));
        assert_eq!(
            "/volume 0".parse::<KillSilenceCommand>(),
            Err(KillSilenceCommandError::InvalidVolume)
        );
        assert_eq!(
            "/volume 11".parse::<KillSilenceCommand>(),
            Err(KillSilenceCommandError::InvalidVolume)
        );
    }

    #[test]
    fn login_never_accepts_a_client_id() {
        assert_eq!(
            "/spotify login anything".parse::<KillSilenceCommand>(),
            Err(KillSilenceCommandError::InvalidSpotifyCommand)
        );
    }

    #[test]
    fn suggestions_are_indexed_and_complete_ambiguous_prefixes() {
        let suggestions = command_suggestions("/pla");
        assert_eq!(
            suggestions
                .iter()
                .map(|item| item.completion)
                .collect::<Vec<_>>(),
            vec!["/play", "/player"]
        );
        assert!(command_suggestions("hello").is_empty());
    }

    #[test]
    fn parses_non_interrupting_screen_commands() {
        assert_eq!("/home".parse(), Ok(KillSilenceCommand::Home));
        assert_eq!("/player".parse(), Ok(KillSilenceCommand::Player));
        assert_eq!("/now-playing".parse(), Ok(KillSilenceCommand::Player));
    }
}
