use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(feature = "image")]
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use rspotify::model::{Id as _, PlayableItem};

use crate::{
    agent_bridge::{
        AgentEvent, AgentProvider, AgentSession, AgentWatch, ClaudeProvider, SessionQuery,
    },
    client::{ClientRequest, PlayerRequest},
    kill_silence_command::{command_suggestions, KillSilenceCommand},
    state::{self, ContextId, Item, Playback, PlaylistFolderItem, SharedState},
};

use super::{
    clean_up,
    kill_silence::{
        render_kill_silence, render_kill_silence_boot, render_kill_silence_home, ArtworkColorMode,
        KillSilenceAgent, KillSilenceAgentStatus, KillSilenceCommandSuggestion, KillSilenceOverlay,
        KillSilenceTrack, KillSilenceViewModel, TerminalArtwork,
    },
    Terminal,
};

const FRAME_INTERVAL: Duration = Duration::from_millis(80);
const PLAYBACK_POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
enum ListEntry {
    Track(state::Track),
    Playlist(state::Playlist),
    Device(state::Device),
    Agent(AgentSession),
    Label(String),
}

impl ListEntry {
    fn label(&self) -> String {
        match self {
            Self::Track(track) => format!("{} — {}", track.artists_info(), track.name),
            Self::Playlist(playlist) => format!("PLAYLIST · {}", playlist.name),
            Self::Device(device) => format!("{} · SPOTIFY CONNECT", device.name),
            Self::Agent(session) => format!("{} · {}", session.project, session.title),
            Self::Label(label) => label.clone(),
        }
    }
}

enum PendingOverlay {
    Search { query: String, requested: Instant },
    Devices { requested: Instant },
    Queue { requested: Instant },
}

enum RuntimeEvent {
    AgentSessions(Result<Vec<AgentSession>, String>),
    AgentWatch(Result<(AgentSession, AgentWatch), String>),
    #[cfg(feature = "image")]
    Artwork {
        track_key: String,
        result: Result<TerminalArtwork, String>,
    },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum KillSilenceScreen {
    #[default]
    Home,
    Player,
}

fn screen_shortcut(code: KeyCode) -> Option<KillSilenceScreen> {
    match code {
        KeyCode::F(1) => Some(KillSilenceScreen::Home),
        KeyCode::F(2) => Some(KillSilenceScreen::Player),
        _ => None,
    }
}

struct Runtime {
    state: SharedState,
    client: flume::Sender<ClientRequest>,
    events_tx: flume::Sender<RuntimeEvent>,
    events_rx: flume::Receiver<RuntimeEvent>,
    provider: Arc<ClaudeProvider>,
    agent_watch: Option<AgentWatch>,
    agent_session: Option<AgentSession>,
    agent_status: KillSilenceAgentStatus,
    agent_started: Option<Instant>,
    agent_completed: Option<Instant>,
    screen: KillSilenceScreen,
    command: String,
    command_suggestion_selected: usize,
    status: String,
    entries: Vec<ListEntry>,
    overlay_title: Option<String>,
    selected: usize,
    pending: Option<PendingOverlay>,
    tick: u64,
    should_quit: bool,
    last_playback_poll: Instant,
    artwork_key: Option<String>,
    artwork: Option<TerminalArtwork>,
}

impl Runtime {
    fn new(state: SharedState, client: flume::Sender<ClientRequest>) -> Self {
        let (events_tx, events_rx) = flume::unbounded();
        Self {
            state,
            client,
            events_tx,
            events_rx,
            provider: Arc::new(ClaudeProvider::default()),
            agent_watch: None,
            agent_session: None,
            agent_status: KillSilenceAgentStatus::Disconnected,
            agent_started: None,
            agent_completed: None,
            screen: KillSilenceScreen::Home,
            command: String::new(),
            command_suggestion_selected: 0,
            status: "COMMAND LINK READY".into(),
            entries: Vec::new(),
            overlay_title: None,
            selected: 0,
            pending: None,
            tick: 0,
            should_quit: false,
            last_playback_poll: Instant::now()
                .checked_sub(PLAYBACK_POLL_INTERVAL)
                .expect("playback interval fits within Instant"),
            artwork_key: None,
            artwork: None,
        }
    }

    fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self.last_playback_poll.elapsed() >= PLAYBACK_POLL_INTERVAL {
            self.send(ClientRequest::GetCurrentPlayback);
            self.last_playback_poll = Instant::now();
        }
        self.drain_runtime_events();
        self.drain_agent_events();
        self.resolve_pending_overlay();
        self.refresh_artwork();
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if let Some(screen) = screen_shortcut(key.code) {
            self.show_screen(screen);
            return;
        }
        if self.overlay_title.is_some() {
            match key.code {
                KeyCode::Esc => self.close_overlay(),
                KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
                }
                KeyCode::Enter => self.activate_selection(),
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.command.clear();
                self.command_suggestion_selected = 0;
            }
            KeyCode::Backspace => {
                self.command.pop();
                self.command_suggestion_selected = 0;
            }
            KeyCode::Up if !self.suggestions().is_empty() => {
                self.command_suggestion_selected =
                    self.command_suggestion_selected.saturating_sub(1);
            }
            KeyCode::Down if !self.suggestions().is_empty() => {
                self.command_suggestion_selected = (self.command_suggestion_selected + 1)
                    .min(self.suggestions().len().saturating_sub(1));
            }
            KeyCode::Tab | KeyCode::Right if !self.suggestions().is_empty() => {
                self.complete_suggestion(false);
            }
            KeyCode::Enter => {
                if self.should_accept_suggestion() {
                    self.complete_suggestion(true);
                } else {
                    self.run_command();
                }
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.command.push(character);
                self.command_suggestion_selected = 0;
            }
            _ => {}
        }
    }

    fn suggestions(&self) -> Vec<crate::kill_silence_command::KillSilenceCommandSpec> {
        command_suggestions(&self.command)
    }

    fn should_accept_suggestion(&self) -> bool {
        let suggestions = self.suggestions();
        let Some(suggestion) = suggestions.get(
            self.command_suggestion_selected
                .min(suggestions.len().saturating_sub(1)),
        ) else {
            return false;
        };
        self.command.trim_end() != suggestion.completion.trim_end()
    }

    fn complete_suggestion(&mut self, run_when_complete: bool) {
        let suggestions = self.suggestions();
        let Some(suggestion) = suggestions
            .get(
                self.command_suggestion_selected
                    .min(suggestions.len().saturating_sub(1)),
            )
            .copied()
        else {
            return;
        };
        self.command = suggestion.completion.into();
        self.command_suggestion_selected = 0;
        if run_when_complete && !suggestion.completion.ends_with(' ') {
            self.run_command();
        }
    }

    fn run_command(&mut self) {
        let input = std::mem::take(&mut self.command);
        self.command_suggestion_selected = 0;
        if !input.trim().is_empty() {
            match input.parse::<KillSilenceCommand>() {
                Ok(command) => self.dispatch(command),
                Err(error) => self.status = error.to_string(),
            }
        }
    }

    fn dispatch(&mut self, command: KillSilenceCommand) {
        match command {
            KillSilenceCommand::Song => self.open_library(),
            KillSilenceCommand::Search(query) => {
                self.send(ClientRequest::Search(query.clone()));
                self.status = format!("SEARCHING SPOTIFY FOR {query}…").to_uppercase();
                self.pending = Some(PendingOverlay::Search {
                    query,
                    requested: Instant::now(),
                });
            }
            KillSilenceCommand::SpotifyDevice => {
                self.send(ClientRequest::GetDevices);
                self.status = "SCANNING SPOTIFY CONNECT DEVICES…".into();
                self.pending = Some(PendingOverlay::Devices {
                    requested: Instant::now(),
                });
            }
            KillSilenceCommand::Queue => {
                self.send(ClientRequest::GetCurrentUserQueue);
                self.status = "READING SPOTIFY QUEUE…".into();
                self.pending = Some(PendingOverlay::Queue {
                    requested: Instant::now(),
                });
            }
            KillSilenceCommand::Play => self.player(PlayerRequest::Resume, "TRANSMISSION RESUMED"),
            KillSilenceCommand::Stop => self.player(PlayerRequest::Pause, "SIGNAL HELD"),
            KillSilenceCommand::Replay => {
                self.send(ClientRequest::Player(PlayerRequest::SeekTrack(
                    chrono::Duration::zero(),
                )));
                self.player(PlayerRequest::Resume, "TRANSMISSION RESTARTED");
            }
            KillSilenceCommand::Next => self.player(PlayerRequest::NextTrack, "NEXT SIGNAL"),
            KillSilenceCommand::Previous => {
                self.player(PlayerRequest::PreviousTrack, "PREVIOUS SIGNAL");
            }
            KillSilenceCommand::Volume(level) => self.player(
                PlayerRequest::Volume(level.saturating_mul(10)),
                &format!("VOLUME SET TO {level:02}/10"),
            ),
            KillSilenceCommand::Like => self.like_current(),
            KillSilenceCommand::WithAgents => self.load_agents(),
            KillSilenceCommand::Home => self.show_screen(KillSilenceScreen::Home),
            KillSilenceCommand::Player => self.show_screen(KillSilenceScreen::Player),
            KillSilenceCommand::Help => self.open_help(),
            KillSilenceCommand::Quit => self.should_quit = true,
        }
    }

    fn player(&mut self, request: PlayerRequest, status: &str) {
        self.send(ClientRequest::Player(request));
        self.status = status.into();
    }

    fn show_screen(&mut self, screen: KillSilenceScreen) {
        self.screen = screen;
        self.close_overlay();
        self.status = match screen {
            KillSilenceScreen::Home => "TITLE SCREEN · SPOTIFY SIGNAL CONTINUES",
            KillSilenceScreen::Player => "NOW PLAYING SCREEN",
        }
        .into();
    }

    fn open_library(&mut self) {
        let data = self.state.data.read();
        let mut entries = data
            .user_data
            .playlists
            .iter()
            .filter_map(|item| match item {
                PlaylistFolderItem::Playlist(playlist) => {
                    Some(ListEntry::Playlist(playlist.clone()))
                }
                PlaylistFolderItem::Folder(_) => None,
            })
            .collect::<Vec<_>>();
        let mut tracks = data
            .user_data
            .saved_tracks
            .values()
            .cloned()
            .collect::<Vec<_>>();
        tracks.sort_by_key(|track| std::cmp::Reverse(track.added_at));
        entries.extend(tracks.into_iter().map(ListEntry::Track));
        drop(data);
        self.open_overlay("SONG//SPOTIFY ARCHIVE", entries);
    }

    fn open_help(&mut self) {
        let labels = [
            "/song · saved tracks and playlists",
            "/search <query> · find Spotify tracks",
            "/spotify device · choose a Connect device",
            "/with-agents · bind an external Claude session",
            "/home · title screen (music keeps playing)",
            "/player · return to the current track",
            "/play /stop /replay /next /prev",
            "/volume 1..10 · set Spotify device volume",
            "/queue /like /quit",
        ];
        self.open_overlay(
            "K/S//COMMAND ARCHIVE",
            labels
                .into_iter()
                .map(|label| ListEntry::Label(label.into()))
                .collect(),
        );
    }

    fn load_agents(&mut self) {
        self.status = "INDEXING EXTERNAL CLAUDE SESSIONS…".into();
        let provider = Arc::clone(&self.provider);
        let tx = self.events_tx.clone();
        tokio::spawn(async move {
            let result = provider
                .list_sessions(SessionQuery {
                    query: None,
                    limit: Some(40),
                })
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send_async(RuntimeEvent::AgentSessions(result)).await;
        });
    }

    fn activate_selection(&mut self) {
        let Some(entry) = self.entries.get(self.selected).cloned() else {
            return;
        };
        match entry {
            ListEntry::Track(_) => {
                let ids = self
                    .entries
                    .iter()
                    .skip(self.selected)
                    .chain(self.entries.iter().take(self.selected))
                    .filter_map(|entry| match entry {
                        ListEntry::Track(track) => Some(state::PlayableId::Track(track.id.clone())),
                        _ => None,
                    })
                    .take(100)
                    .collect::<Vec<_>>();
                self.send(ClientRequest::Player(PlayerRequest::StartPlayback(
                    Playback::URIs(ids, None),
                    None,
                )));
                self.status = "SPOTIFY TRANSMISSION STARTED".into();
                self.screen = KillSilenceScreen::Player;
            }
            ListEntry::Playlist(playlist) => {
                self.send(ClientRequest::Player(PlayerRequest::StartPlayback(
                    Playback::Context(ContextId::Playlist(playlist.id), None),
                    None,
                )));
                self.status = "PLAYLIST TRANSMISSION STARTED".into();
                self.screen = KillSilenceScreen::Player;
            }
            ListEntry::Device(device) => {
                self.send(ClientRequest::Player(PlayerRequest::TransferPlayback(
                    device.id, true,
                )));
                self.status = format!("DEVICE LINKED: {}", device.name).to_uppercase();
            }
            ListEntry::Agent(session) => self.bind_agent(session),
            ListEntry::Label(_) => {}
        }
        self.close_overlay();
    }

    fn bind_agent(&mut self, session: AgentSession) {
        self.status = "ARMING EXTERNAL CLAUDE LINK…".into();
        let provider = Arc::clone(&self.provider);
        let tx = self.events_tx.clone();
        tokio::spawn(async move {
            let result = provider
                .watch(session.clone())
                .await
                .map(|watch| (session, watch))
                .map_err(|error| error.to_string());
            let _ = tx.send_async(RuntimeEvent::AgentWatch(result)).await;
        });
    }

    fn like_current(&mut self) {
        let full = {
            let player = self.state.player.read();
            match player.currently_playing() {
                Some(PlayableItem::Track(track)) => Some(track.clone()),
                _ => None,
            }
        };
        match full.and_then(state::Track::try_from_full_track) {
            Some(track) => {
                self.send(ClientRequest::AddToLibrary(Item::Track(track)));
                self.status = "TRACK SAVED TO SPOTIFY LIBRARY".into();
            }
            None => self.status = "NO CURRENT TRACK TO SAVE".into(),
        }
    }

    fn drain_runtime_events(&mut self) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                RuntimeEvent::AgentSessions(result) => match result {
                    Ok(sessions) => self.open_overlay(
                        "CLAUDE//CODE · SESSION ARCHIVE",
                        sessions.into_iter().map(ListEntry::Agent).collect(),
                    ),
                    Err(error) => self.status = short_error(&error),
                },
                RuntimeEvent::AgentWatch(result) => match result {
                    Ok((session, watch)) => {
                        self.agent_watch = Some(watch);
                        self.agent_session = Some(session);
                        self.agent_status = KillSilenceAgentStatus::Armed;
                        self.agent_completed = None;
                        self.status = "EXTERNAL CLAUDE LINK ARMED".into();
                    }
                    Err(error) => {
                        self.agent_status = KillSilenceAgentStatus::Error;
                        self.agent_completed = None;
                        self.status = short_error(&error);
                    }
                },
                #[cfg(feature = "image")]
                RuntimeEvent::Artwork { track_key, result } => {
                    if self.artwork_key.as_deref() == Some(track_key.as_str()) {
                        match result {
                            Ok(artwork) => self.artwork = Some(artwork),
                            Err(error) => tracing::warn!("Album artwork unavailable: {error}"),
                        }
                    }
                }
            }
        }
    }

    fn drain_agent_events(&mut self) {
        loop {
            let event = self
                .agent_watch
                .as_mut()
                .and_then(|watch| watch.try_recv().ok());
            let Some(event) = event else {
                break;
            };
            match event {
                AgentEvent::Watching { .. } => {
                    self.agent_status = KillSilenceAgentStatus::Armed;
                    self.agent_completed = None;
                    self.status = "WATCHING EXTERNAL CLAUDE TERMINAL".into();
                }
                AgentEvent::TurnStarted { .. } => {
                    self.agent_status = KillSilenceAgentStatus::Working;
                    self.agent_started = Some(Instant::now());
                    self.agent_completed = None;
                    self.player(
                        PlayerRequest::Resume,
                        "EXTERNAL CLAUDE · WORKING · SOUNDTRACK ACTIVE",
                    );
                }
                AgentEvent::TurnCompleted { .. } => {
                    self.agent_status = KillSilenceAgentStatus::Complete;
                    self.agent_completed = Some(Instant::now());
                    self.player(
                        PlayerRequest::Pause,
                        "EXTERNAL CLAUDE TURN COMPLETE · SOUNDTRACK HELD",
                    );
                }
                AgentEvent::Interrupted { .. } => {
                    self.agent_status = KillSilenceAgentStatus::Interrupted;
                    self.agent_completed = None;
                    self.player(PlayerRequest::Pause, "EXTERNAL CLAUDE TURN INTERRUPTED");
                }
                AgentEvent::Error { message, .. } => {
                    self.agent_status = KillSilenceAgentStatus::Error;
                    self.agent_completed = None;
                    self.status = short_error(&message);
                }
            }
        }
    }

    fn resolve_pending_overlay(&mut self) {
        let ready = match self.pending.as_ref() {
            Some(PendingOverlay::Search { query, requested }) => {
                let tracks = self
                    .state
                    .data
                    .read()
                    .caches
                    .search
                    .get(query)
                    .map(|results| results.tracks.clone());
                tracks
                    .map(|tracks| {
                        (
                            "SEARCH//SPOTIFY",
                            tracks.into_iter().map(ListEntry::Track).collect(),
                        )
                    })
                    .or_else(|| elapsed_empty(requested, "SEARCH//SPOTIFY"))
            }
            Some(PendingOverlay::Devices { requested }) => {
                let devices = self.state.player.read().devices.clone();
                if devices.is_empty() {
                    elapsed_empty(requested, "SPOTIFY//CONNECT DEVICES")
                } else {
                    Some((
                        "SPOTIFY//CONNECT DEVICES",
                        devices.into_iter().map(ListEntry::Device).collect(),
                    ))
                }
            }
            Some(PendingOverlay::Queue { requested }) => {
                let queue = self.state.player.read().queue.clone();
                match queue {
                    Some(queue) => Some((
                        "SPOTIFY//QUEUE",
                        queue
                            .queue
                            .into_iter()
                            .filter_map(|item| match item {
                                PlayableItem::Track(track) => {
                                    state::Track::try_from_full_track(track).map(ListEntry::Track)
                                }
                                _ => None,
                            })
                            .collect(),
                    )),
                    None => elapsed_empty(requested, "SPOTIFY//QUEUE"),
                }
            }
            None => None,
        };
        if let Some((title, entries)) = ready {
            self.pending = None;
            self.open_overlay(title, entries);
        }
    }

    fn open_overlay(&mut self, title: &str, entries: Vec<ListEntry>) {
        self.overlay_title = Some(title.into());
        self.entries = entries;
        self.selected = 0;
        self.status = if self.entries.is_empty() {
            "NO SIGNALS FOUND".into()
        } else {
            "SELECT A SIGNAL".into()
        };
    }

    fn close_overlay(&mut self) {
        self.overlay_title = None;
        self.entries.clear();
        self.selected = 0;
    }

    fn send(&mut self, request: ClientRequest) {
        if let Err(error) = self.client.send(request) {
            self.status = short_error(&error.to_string());
        }
    }

    fn view_model(&self) -> KillSilenceViewModel {
        let data = self.state.data.read();
        let account = data.user_data.user.as_ref().map(|user| {
            user.display_name
                .clone()
                .unwrap_or_else(|| user.id.id().to_owned())
        });
        drop(data);
        let player = self.state.player.read();
        let playback = player.current_playback();
        let (track, artwork_url, track_key) = playback
            .as_ref()
            .and_then(|playback| playback.item.as_ref())
            .map_or((None, None, None), playable_view);
        let progress_ms = player
            .playback_progress()
            .map_or(0, |duration| duration.num_milliseconds().max(0) as u64);
        let is_playing = playback
            .as_ref()
            .is_some_and(|playback| playback.is_playing);
        let volume = playback
            .as_ref()
            .and_then(|playback| playback.device.volume_percent)
            .unwrap_or(70)
            .min(100) as u8;
        let device = playback.as_ref().map_or_else(
            || "NO ACTIVE DEVICE".into(),
            |playback| playback.device.name.clone(),
        );
        drop(player);
        let _ = (artwork_url, track_key);
        let overlay = self.overlay_title.as_ref().map(|title| KillSilenceOverlay {
            title: title.clone(),
            items: self.entries.iter().map(ListEntry::label).collect(),
            selected: self.selected,
            footer: "↑↓ SELECT · ENTER OPEN · ESC CLOSE".into(),
        });
        let (project, session_title) = self.agent_session.as_ref().map_or_else(
            || (String::new(), String::new()),
            |session| (session.project.clone(), session.title.clone()),
        );
        let suggestions = self.suggestions();
        KillSilenceViewModel {
            account_label: account.clone().unwrap_or_else(|| "SPOTIFY LINKED".into()),
            authenticated: account.is_some(),
            status_line: self.status.clone(),
            track,
            progress_ms,
            is_playing,
            volume_percent: volume,
            device_name: device,
            command_line: self.command.clone(),
            command_suggestions: suggestions
                .iter()
                .map(|suggestion| KillSilenceCommandSuggestion {
                    usage: suggestion.usage.into(),
                    description: suggestion.description.into(),
                })
                .collect(),
            command_suggestion_selected: self
                .command_suggestion_selected
                .min(suggestions.len().saturating_sub(1)),
            agent: KillSilenceAgent {
                status: self.agent_status,
                project,
                session_title,
                elapsed_seconds: self
                    .agent_started
                    .map_or(0, |started| started.elapsed().as_secs()),
                completion_elapsed_ms: self
                    .agent_completed
                    .map(|completed| completed.elapsed().as_millis() as u64),
            },
            overlay,
            frame_tick: self.tick,
            artwork_color_mode: ArtworkColorMode::Auto,
        }
    }

    fn refresh_artwork(&mut self) {
        let current = {
            let player = self.state.player.read();
            player
                .current_playback()
                .and_then(|playback| playback.item)
                .and_then(|item| {
                    let (_, url, key) = playable_view(&item);
                    key.zip(url)
                })
        };
        let key = current.as_ref().map(|(key, _)| key.clone());
        if key == self.artwork_key {
            return;
        }
        self.artwork_key = key;
        self.artwork = None;
        #[cfg(feature = "image")]
        if let Some((track_key, url)) = current {
            let tx = self.events_tx.clone();
            tokio::spawn(async move {
                let result = download_artwork(&track_key, &url).await;
                let _ = tx
                    .send_async(RuntimeEvent::Artwork { track_key, result })
                    .await;
            });
        }
    }
}

pub(crate) fn run_kill_silence(
    state: SharedState,
    client: flume::Sender<ClientRequest>,
    mut terminal: Terminal,
) -> Result<()> {
    let boot_started = Instant::now();
    while boot_started.elapsed() < Duration::from_millis(650) {
        let tick = (boot_started.elapsed().as_millis() / 80) as u64;
        terminal.draw(|frame| render_kill_silence_boot(frame, tick))?;
        std::thread::sleep(FRAME_INTERVAL);
    }
    let mut runtime = Runtime::new(state, client);
    let mut last_draw = Instant::now()
        .checked_sub(FRAME_INTERVAL)
        .expect("frame interval fits within Instant");
    while !runtime.should_quit {
        if crossterm::event::poll(Duration::from_millis(25))? {
            if let Event::Key(key) = crossterm::event::read()? {
                runtime.handle_key(key);
            }
        }
        runtime.tick();
        if last_draw.elapsed() >= FRAME_INTERVAL {
            let model = runtime.view_model();
            terminal.draw(|frame| match runtime.screen {
                KillSilenceScreen::Home => render_kill_silence_home(frame, &model),
                KillSilenceScreen::Player => {
                    render_kill_silence(frame, &model, runtime.artwork.as_ref());
                }
            })?;
            last_draw = Instant::now();
        }
    }
    clean_up(terminal).context("restore terminal after KILL//SILENCE")
}

fn playable_view(
    item: &PlayableItem,
) -> (Option<KillSilenceTrack>, Option<String>, Option<String>) {
    match item {
        PlayableItem::Track(track) => {
            let key = track
                .id
                .as_ref()
                .map_or_else(|| track.name.clone(), |id| id.id().to_owned());
            let artwork = track.album.images.first().map(|image| image.url.clone());
            (
                Some(KillSilenceTrack {
                    title: track.name.clone(),
                    artists: track
                        .artists
                        .iter()
                        .map(|artist| artist.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    album: track.album.name.clone(),
                    duration_ms: track.duration.num_milliseconds().max(0) as u64,
                }),
                artwork,
                Some(key),
            )
        }
        PlayableItem::Episode(episode) => (
            Some(KillSilenceTrack {
                title: episode.name.clone(),
                artists: episode.show.publisher.clone(),
                album: episode.show.name.clone(),
                duration_ms: episode.duration.num_milliseconds().max(0) as u64,
            }),
            episode.images.first().map(|image| image.url.clone()),
            Some(episode.id.id().to_owned()),
        ),
        PlayableItem::Unknown(_) => (None, None, None),
    }
}

fn elapsed_empty(
    requested: &Instant,
    title: &'static str,
) -> Option<(&'static str, Vec<ListEntry>)> {
    (requested.elapsed() >= Duration::from_millis(900)).then_some((title, Vec::new()))
}

fn short_error(error: &str) -> String {
    let one_line = error.replace(['\r', '\n'], " ");
    if one_line.chars().count() <= 110 {
        one_line.to_uppercase()
    } else {
        format!("{}…", one_line.chars().take(110).collect::<String>()).to_uppercase()
    }
}

#[cfg(feature = "image")]
async fn download_artwork(track_key: &str, url: &str) -> Result<TerminalArtwork, String> {
    let directory = crate::config::get_config()
        .cache_folder
        .join("kill-silence-artwork");
    let safe_key = track_key
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let path: PathBuf = directory.join(format!("{safe_key}.image"));
    let bytes = if let Ok(bytes) = std::fs::read(&path) {
        bytes
    } else {
        let response = reqwest::get(url).await.map_err(|error| error.to_string())?;
        let bytes = response
            .error_for_status()
            .map_err(|error| error.to_string())?
            .bytes()
            .await
            .map_err(|error| error.to_string())?
            .to_vec();
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        std::fs::write(&path, &bytes).map_err(|error| error.to_string())?;
        bytes
    };
    let image = image::load_from_memory(&bytes).map_err(|error| error.to_string())?;
    Ok(TerminalArtwork::from_image(&image, 36, 16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_errors_are_single_line_and_bounded() {
        let result = short_error(&format!("first\n{}", "x".repeat(200)));
        assert!(!result.contains('\n'));
        assert!(result.chars().count() <= 111);
    }

    #[test]
    fn list_labels_never_include_agent_transcript_content() {
        let entry = ListEntry::Agent(AgentSession {
            id: "id".into(),
            project: "kill-silence".into(),
            title: "current coding turn".into(),
            transcript: std::path::PathBuf::from("secret.jsonl"),
            updated_at: std::time::SystemTime::now(),
        });
        let label = entry.label();
        assert!(!label.contains("secret.jsonl"));
    }

    #[test]
    fn function_keys_map_to_non_interrupting_screen_changes() {
        assert_eq!(
            screen_shortcut(KeyCode::F(1)),
            Some(KillSilenceScreen::Home)
        );
        assert_eq!(
            screen_shortcut(KeyCode::F(2)),
            Some(KillSilenceScreen::Player)
        );
        assert_eq!(screen_shortcut(KeyCode::Char('p')), None);
    }
}
