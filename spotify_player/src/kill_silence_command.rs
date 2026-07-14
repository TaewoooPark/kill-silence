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
    Help,
    Quit,
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
}
