//! KILL//SILENCE's standalone Ratatui presentation layer.
//!
//! The renderer deliberately depends only on [`KillSilenceViewModel`]. Spotify,
//! authentication and agent-watcher state should be adapted to this model by the
//! application's integration layer.

use std::{borrow::Cow, sync::OnceLock};

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

const WIDE_LAYOUT_AT: u16 = 120;
const SIGNAL_MAX: u16 = 8;
const AGENT_BLINK_HALF_PERIOD_MS: u64 = 300;
const AGENT_BLINK_COUNT: u64 = 10;

const WIDE_LOGO: &[&str] = &[
    "██╗  ██╗  ██╗  ██╗      ██╗          ███████╗  ██╗  ██╗      ███████╗  ███╗   ██╗  ██████╗  ███████╗",
    "██║ ██╔╝  ██║  ██║      ██║          ██╔════╝  ██║  ██║      ██╔════╝  ████╗  ██║  ██╔════╝  ██╔════╝",
    "█████╔╝   ██║  ██║      ██║   //     ███████╗  ██║  ██║      █████╗    ██╔██╗ ██║  ██║       █████╗",
    "██╔═██╗   ██║  ██║      ██║          ╚════██║  ██║  ██║      ██╔══╝    ██║╚██╗██║  ██║       ██╔══╝",
    "██║  ██╗  ██║  ███████╗ ███████╗     ███████║  ██║  ███████╗ ███████╗ ██║ ╚████║  ╚██████╗  ███████╗",
    "╚═╝  ╚═╝  ╚═╝  ╚══════╝ ╚══════╝     ╚══════╝  ╚═╝  ╚══════╝ ╚══════╝ ╚═╝  ╚═══╝   ╚═════╝  ╚══════╝",
];

const COMPACT_LOGO: &[&str] = &[
    "█ █  ███  █    █       ╱╱      ███  █  █    ███  █  █  ███  ███",
    "██    █   █    █      ╱╱       █    █  █    █    ██ █  █    █",
    "█ █  ███  ███  ███   ╱╱       ███  █  ███  ███  █ ██  ███  ███",
];

// Chrome always uses xterm-256 colours. This prevents Terminal.app from turning
// an unsupported RGB background into a white canvas.
const VOID: Color = Color::Indexed(232);
const PANEL: Color = Color::Indexed(233);
const CYAN: Color = Color::Indexed(51);
const MAGENTA: Color = Color::Indexed(200);
const GREEN: Color = Color::Indexed(82);
const AMBER: Color = Color::Indexed(220);
const TEXT: Color = Color::Indexed(252);
const MUTED: Color = Color::Indexed(244);
const DIM: Color = Color::Indexed(238);

/// Fixed visual signal. It is intentionally decorative and never audio-derived.
pub const SIGNAL_PRESET: &[u8] = &[
    2, 3, 4, 4, 3, 3, 2, 2, 2, 3, 5, 7, 6, 4, 3, 3, 2, 2, 2, 1, 1, 3, 4, 5, 4, 3, 2, 2, 3, 4, 4, 3,
    2, 2, 2, 3, 5, 6, 7, 5, 4, 4, 3, 2, 2, 1, 2, 3, 3, 4, 6, 8, 7, 6, 5, 4, 3, 3, 2, 2, 3, 4, 5, 5,
    4, 3, 2, 2, 2, 3, 4, 6, 6, 5, 4, 3, 3, 2, 2, 1, 3, 5, 7, 8, 6, 4, 3, 3, 2, 2, 3, 4, 5, 4, 3, 2,
    2, 3, 4, 5, 7, 6, 5, 4, 3, 2, 2, 2, 3, 5, 6, 5, 4, 3, 3, 2, 2, 3, 4, 6, 8, 7, 5, 4, 3, 3, 2, 2,
];

/// Terminal colour behaviour for album artwork.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ArtworkColorMode {
    /// Inspect `COLORTERM`; unknown terminals receive the safe xterm-256 path.
    #[default]
    Auto,
    /// Keep the source RGB colours.
    TrueColor,
    /// Quantize source RGB colours to xterm-256.
    Ansi256,
}

/// A single upper-half-block artwork cell (top pixel is foreground, bottom is background).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtworkCell {
    pub glyph: char,
    pub foreground: Color,
    pub background: Color,
}

/// Album artwork pre-rendered at terminal-cell resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalArtwork {
    width: u16,
    height: u16,
    cells: Vec<ArtworkCell>,
}

impl TerminalArtwork {
    /// Construct artwork from row-major terminal cells.
    #[must_use]
    pub fn new(width: u16, height: u16, cells: Vec<ArtworkCell>) -> Self {
        let expected = usize::from(width) * usize::from(height);
        let mut cells = cells;
        cells.truncate(expected);
        cells.resize(
            expected,
            ArtworkCell {
                glyph: ' ',
                foreground: VOID,
                background: VOID,
            },
        );
        Self {
            width,
            height,
            cells,
        }
    }

    /// Build the deterministic K/S placeholder used while cover art is unavailable.
    #[must_use]
    pub fn placeholder(width: u16, height: u16) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
        for y in 0..height {
            for x in 0..width {
                let edge = x == 0 || y == 0 || x + 1 == width || y + 1 == height;
                let cross = x == width / 2 || y == height / 2;
                let pulse = (u32::from(x) * 17 + u32::from(y) * 29) % 9 < 3;
                let (foreground, background) = if edge {
                    (Color::Indexed(51), Color::Indexed(51))
                } else if cross {
                    (Color::Indexed(200), Color::Indexed(89))
                } else if pulse {
                    (Color::Indexed(30), Color::Indexed(23))
                } else {
                    (Color::Indexed(234), Color::Indexed(232))
                };
                cells.push(ArtworkCell {
                    glyph: '▀',
                    foreground,
                    background,
                });
            }
        }
        Self::new(width, height, cells)
    }

    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }

    #[must_use]
    pub fn cell(&self, x: u16, y: u16) -> Option<ArtworkCell> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.cells
            .get(usize::from(y) * usize::from(self.width) + usize::from(x))
            .copied()
    }

    /// Decode an image into true-colour upper-half-block cells.
    ///
    /// Each terminal row stores two source pixels. Colour reduction, when needed,
    /// happens at render time so the same cached artwork works in every terminal.
    #[cfg(feature = "image")]
    #[must_use]
    pub fn from_image(image: &image::DynamicImage, max_width: u16, max_height: u16) -> Self {
        use image::{imageops::FilterType, GenericImageView};

        if max_width == 0 || max_height == 0 || image.width() == 0 || image.height() == 0 {
            return Self::new(0, 0, Vec::new());
        }
        let resized = image.resize(
            u32::from(max_width),
            u32::from(max_height) * 2,
            FilterType::Triangle,
        );
        let width = resized.width().min(u32::from(u16::MAX)) as u16;
        let height = resized.height().div_ceil(2).min(u32::from(u16::MAX)) as u16;
        let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
        for row in 0..u32::from(height) {
            let top_y = (row * 2).min(resized.height() - 1);
            let bottom_y = (top_y + 1).min(resized.height() - 1);
            for x in 0..u32::from(width) {
                let top = resized.get_pixel(x, top_y).0;
                let bottom = resized.get_pixel(x, bottom_y).0;
                cells.push(ArtworkCell {
                    glyph: '▀',
                    foreground: Color::Rgb(top[0], top[1], top[2]),
                    background: Color::Rgb(bottom[0], bottom[1], bottom[2]),
                });
            }
        }
        Self::new(width, height, cells)
    }
}

/// Metadata shown in the transmission panel.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KillSilenceTrack {
    pub title: String,
    pub artists: String,
    pub album: String,
    pub duration_ms: u64,
}

/// State of the externally watched coding-agent session.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum KillSilenceAgentStatus {
    #[default]
    Disconnected,
    Armed,
    Working,
    Complete,
    Interrupted,
    Error,
}

/// Agent information intentionally excludes prompt and response content.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KillSilenceAgent {
    pub status: KillSilenceAgentStatus,
    pub project: String,
    pub session_title: String,
    pub elapsed_seconds: u64,
    /// Time since completion. `None` means the completion banner is already solid.
    pub completion_elapsed_ms: Option<u64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KillSilenceCommandSuggestion {
    pub usage: String,
    pub description: String,
}

/// A modal list supplied by the integration layer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KillSilenceOverlay {
    pub title: String,
    pub items: Vec<String>,
    pub selected: usize,
    pub footer: String,
}

/// Complete, side-effect-free input to the KILL//SILENCE renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillSilenceViewModel {
    pub account_label: String,
    pub authenticated: bool,
    pub status_line: String,
    pub track: Option<KillSilenceTrack>,
    pub progress_ms: u64,
    pub is_playing: bool,
    pub volume_percent: u8,
    pub device_name: String,
    pub command_line: String,
    pub command_suggestions: Vec<KillSilenceCommandSuggestion>,
    pub command_suggestion_selected: usize,
    pub agent: KillSilenceAgent,
    pub overlay: Option<KillSilenceOverlay>,
    pub frame_tick: u64,
    pub artwork_color_mode: ArtworkColorMode,
}

impl Default for KillSilenceViewModel {
    fn default() -> Self {
        Self {
            account_label: "SPOTIFY OFFLINE".into(),
            authenticated: false,
            status_line: "NO SIGNAL".into(),
            track: None,
            progress_ms: 0,
            is_playing: false,
            volume_percent: 70,
            device_name: "NO ACTIVE DEVICE".into(),
            command_line: String::new(),
            command_suggestions: Vec::new(),
            command_suggestion_selected: 0,
            agent: KillSilenceAgent::default(),
            overlay: None,
            frame_tick: 0,
            artwork_color_mode: ArtworkColorMode::Auto,
        }
    }
}

/// Render the persistent title screen without affecting playback state.
pub fn render_kill_silence_home(frame: &mut Frame<'_>, model: &KillSilenceViewModel) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::new().fg(TEXT).bg(VOID)), area);
    if area.width >= WIDE_LAYOUT_AT {
        let columns = Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
            .spacing(1)
            .split(area);
        render_home_panel(frame, columns[0], model);
        render_agent(frame, columns[1], model);
    } else {
        let agent_height = if area.height >= 24 { 7 } else { 6 };
        let rows = Layout::vertical([Constraint::Min(12), Constraint::Length(agent_height)])
            .spacing(1)
            .split(area);
        render_home_panel(frame, rows[0], model);
        render_agent(frame, rows[1], model);
    }
    if let Some(overlay) = &model.overlay {
        render_overlay(frame, overlay);
    }
}

/// Render the KILL//SILENCE player, agent panel, command line and optional overlay.
pub fn render_kill_silence(
    frame: &mut Frame<'_>,
    model: &KillSilenceViewModel,
    artwork: Option<&TerminalArtwork>,
) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::new().fg(TEXT).bg(VOID)), area);

    if area.width >= WIDE_LAYOUT_AT {
        let columns = Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
            .spacing(1)
            .split(area);
        render_player(frame, columns[0], model, artwork);
        render_agent(frame, columns[1], model);
    } else {
        let agent_height = if area.height >= 24 { 7 } else { 6 };
        let rows = Layout::vertical([Constraint::Min(12), Constraint::Length(agent_height)])
            .spacing(1)
            .split(area);
        render_player(frame, rows[0], model, artwork);
        render_agent(frame, rows[1], model);
    }

    if let Some(overlay) = &model.overlay {
        render_overlay(frame, overlay);
    }
}

/// Render the static retro-futuristic startup transmission.
pub fn render_kill_silence_boot(frame: &mut Frame<'_>, tick: u64) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::new().fg(TEXT).bg(VOID)), area);
    let panel = centered_rect(area, 94, 88);
    let block = Block::bordered()
        .border_style(Style::new().fg(CYAN))
        .style(Style::new().fg(TEXT).bg(PANEL));
    let inner = block.inner(panel).inner(Margin::new(2, 1));
    frame.render_widget(block, panel);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(7),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("KILL//SILENCE", title_style()),
            Span::styled("   NULL AUDIO DIVISION // SPOTIFY LINK", muted_style()),
        ]))
        .style(panel_style()),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(
            WIDE_LOGO
                .iter()
                .map(|line| Line::styled(*line, title_style()))
                .collect::<Vec<_>>(),
        )
        .alignment(Alignment::Center)
        .style(panel_style()),
        rows[1],
    );
    let pulse = if tick % 8 < 4 {
        "● SIGNAL"
    } else {
        "○ SIGNAL"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(pulse, title_style()),
            Span::styled("     FREQ://∞     ", muted_style()),
            Span::styled("SILENCE ●", accent_style()),
        ]))
        .alignment(Alignment::Center)
        .style(panel_style()),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("TRANSMISSION BEGINS WHERE SILENCE ENDS", accent_style()),
            Line::styled("LINKING SPOTIFY CONTROL SURFACE…", Style::new().fg(GREEN)),
        ])
        .alignment(Alignment::Center)
        .style(panel_style()),
        rows[3],
    );
}

fn render_home_panel(frame: &mut Frame<'_>, area: Rect, model: &KillSilenceViewModel) {
    if area.width < 4 || area.height < 4 {
        return;
    }
    let block = Block::bordered()
        .border_style(Style::new().fg(CYAN))
        .style(panel_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let compact = inner.height < 22 || inner.width < 112;
    let visible = if compact { 2 } else { 5 }.min(model.command_suggestions.len());
    let command_height = 1 + visible as u16;
    let logo_height = if compact { 3 } else { 6 };
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(logo_height),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(command_height),
    ])
    .split(inner);
    render_header(frame, rows[0], model);
    let logo = if inner.width < 112 {
        COMPACT_LOGO
    } else {
        WIDE_LOGO
    };
    frame.render_widget(
        Paragraph::new(
            logo.iter()
                .map(|line| Line::styled(*line, title_style()))
                .collect::<Vec<_>>(),
        )
        .alignment(Alignment::Center)
        .style(panel_style()),
        rows[1],
    );
    let pulse = if model.frame_tick % 8 < 4 {
        "● SIGNAL"
    } else {
        "○ SIGNAL"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(pulse, title_style()),
            Span::styled(
                "     TRANSMISSION BEGINS WHERE SILENCE ENDS     ",
                accent_style(),
            ),
            Span::styled("FREQ://∞", muted_style()),
        ]))
        .alignment(Alignment::Center)
        .style(panel_style()),
        rows[2],
    );
    render_status(frame, rows[3], model);
    render_command(frame, rows[4], model, visible);
}

fn render_player(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &KillSilenceViewModel,
    artwork: Option<&TerminalArtwork>,
) {
    if area.width < 4 || area.height < 4 {
        return;
    }
    let block = Block::bordered()
        .border_style(Style::new().fg(CYAN))
        .style(panel_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let compact = inner.height < 20;
    let visible = if compact { 2 } else { 5 }.min(model.command_suggestions.len());
    let command_height = 1 + visible as u16;
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(if compact { 5 } else { 8 }),
        Constraint::Min(if compact { 4 } else { 7 }),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(command_height),
    ])
    .split(inner);
    render_header(frame, rows[0], model);
    render_track(frame, rows[1], model, artwork);
    render_signal(frame, rows[2], model);
    render_progress(frame, rows[3], model);
    render_status(frame, rows[4], model);
    render_command(frame, rows[5], model, visible);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, model: &KillSilenceViewModel) {
    let account_color = if model.authenticated { GREEN } else { AMBER };
    let rule_width = usize::from(area.width.saturating_sub(2));
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" KILL//SILENCE", title_style()),
                Span::styled("  ·  K/S SPOTIFY TERMINAL  ", muted_style()),
                Span::styled(
                    model.account_label.to_uppercase(),
                    Style::new().fg(account_color),
                ),
            ]),
            Line::styled(format!(" {}", "─".repeat(rule_width)), Style::new().fg(DIM)),
        ])
        .style(panel_style()),
        area,
    );
}

fn render_track(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &KillSilenceViewModel,
    artwork: Option<&TerminalArtwork>,
) {
    if area.width < 8 || area.height == 0 {
        return;
    }
    let art_width = if area.width >= 70 {
        area.height.saturating_mul(2).min(24)
    } else {
        16
    };
    let columns = Layout::horizontal([Constraint::Length(art_width), Constraint::Min(8)])
        .spacing(2)
        .split(area);
    render_artwork(frame, columns[0], artwork, model.artwork_color_mode);

    let fallback = KillSilenceTrack {
        title: "NO SIGNAL".into(),
        artists: "CONNECT SPOTIFY TO BEGIN".into(),
        album: "—".into(),
        duration_ms: 0,
    };
    let track = model.track.as_ref().unwrap_or(&fallback);
    let playback = if model.is_playing {
        "SIGNAL TRANSMISSION IN PROGRESS"
    } else {
        "SIGNAL HELD"
    };
    let lines = vec![
        Line::styled(
            track.title.to_uppercase(),
            Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
        Line::styled(track.artists.to_uppercase(), title_style()),
        Line::styled(track.album.to_uppercase(), muted_style()),
        Line::from(vec![
            Span::styled("STATUS  ", muted_style()),
            Span::styled(
                playback,
                Style::new()
                    .fg(if model.is_playing { CYAN } else { AMBER })
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("DEVICE  ", muted_style()),
            Span::styled(model.device_name.as_str(), Style::new().fg(TEXT)),
        ]),
        Line::from(vec![
            Span::styled("LENGTH  ", muted_style()),
            Span::styled(format_time(track.duration_ms), Style::new().fg(MAGENTA)),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(panel_style()),
        columns[1],
    );
}

fn render_artwork(
    frame: &mut Frame<'_>,
    area: Rect,
    artwork: Option<&TerminalArtwork>,
    color_mode: ArtworkColorMode,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let fallback;
    let artwork = if let Some(artwork) = artwork.filter(|art| art.width() > 0 && art.height() > 0) {
        artwork
    } else {
        fallback = TerminalArtwork::placeholder(area.width, area.height);
        &fallback
    };
    let height = artwork.height().min(area.height);
    let width = artwork.width().min(area.width);
    let mut lines = Vec::with_capacity(usize::from(height));
    for y in 0..height {
        let spans = (0..width)
            .filter_map(|x| artwork.cell(x, y))
            .map(|cell| {
                Span::styled(
                    cell.glyph.to_string(),
                    Style::new()
                        .fg(terminal_color(cell.foreground, color_mode))
                        .bg(terminal_color(cell.background, color_mode)),
                )
            })
            .collect::<Vec<_>>();
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines).style(Style::new().bg(VOID)), area);
}

fn render_signal(frame: &mut Frame<'_>, area: Rect, model: &KillSilenceViewModel) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    frame.render_widget(
        Paragraph::new(" LIVE SIGNAL WAVEFORM // PRESET STREAM")
            .style(panel_style().fg(MAGENTA).add_modifier(Modifier::BOLD)),
        Rect { height: 1, ..area },
    );
    let graph = Rect {
        y: area.y.saturating_add(1),
        height: area.height.saturating_sub(1),
        ..area
    };
    let heights = signal_heights_at(graph.width, (model.progress_ms / 120) as usize);
    let split = (playback_ratio(model) * f64::from(graph.width)).round() as usize;
    let mut lines = Vec::with_capacity(usize::from(graph.height));
    for row in 0..graph.height {
        let threshold = graph.height - row;
        let spans = heights
            .iter()
            .copied()
            .enumerate()
            .map(|(column, height)| {
                let scaled = ((u16::from(height) * graph.height).div_ceil(SIGNAL_MAX)).max(1);
                let glyph = if scaled >= threshold { "█" } else { " " };
                let color = if column < split {
                    CYAN
                } else {
                    Color::Indexed(242)
                };
                Span::styled(glyph, Style::new().fg(color).bg(PANEL))
            })
            .collect::<Vec<_>>();
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines).style(panel_style()), graph);
}

fn render_progress(frame: &mut Frame<'_>, area: Rect, model: &KillSilenceViewModel) {
    let duration = model.track.as_ref().map_or(0, |track| track.duration_ms);
    let label = format!(
        " {}  {}  {} ",
        format_time(model.progress_ms),
        if model.is_playing { "▶" } else { "Ⅱ" },
        format_time(duration)
    );
    frame.render_widget(
        Gauge::default()
            .ratio(playback_ratio(model))
            .label(label)
            .gauge_style(Style::new().fg(CYAN).bg(DIM).add_modifier(Modifier::BOLD)),
        area,
    );
}

fn render_status(frame: &mut Frame<'_>, area: Rect, model: &KillSilenceViewModel) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" STATUS  ", accent_style()),
            Span::styled(
                model.status_line.to_uppercase(),
                Style::new().fg(GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "   DEVICE {}   VOL {:02}/10",
                    model.device_name,
                    (u16::from(model.volume_percent.min(100)) + 5) / 10
                ),
                muted_style(),
            ),
        ]))
        .style(panel_style()),
        area,
    );
}

fn render_command(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &KillSilenceViewModel,
    visible_suggestions: usize,
) {
    let command = if model.command_line.is_empty() {
        Cow::Borrowed("type /help")
    } else {
        Cow::Borrowed(model.command_line.as_str())
    };
    let mut command_spans = vec![
        Span::styled(" ks:// ", accent_style()),
        Span::styled("█ ", Style::new().fg(CYAN)),
        Span::styled(command, muted_style()),
    ];
    if visible_suggestions > 0 {
        command_spans.push(Span::styled("   ↑↓ INDEX · TAB COMPLETE", muted_style()));
    }
    let mut lines = vec![Line::from(command_spans)];
    let selected = model
        .command_suggestion_selected
        .min(model.command_suggestions.len().saturating_sub(1));
    let suggestion_start = if visible_suggestions > 0 && selected >= visible_suggestions {
        selected + 1 - visible_suggestions
    } else {
        0
    };
    lines.extend(
        model
            .command_suggestions
            .iter()
            .enumerate()
            .skip(suggestion_start)
            .take(visible_suggestions)
            .map(|(index, suggestion)| {
                let selected = index == model.command_suggestion_selected;
                Line::from(vec![
                    Span::styled(
                        if selected { "   ▸ " } else { "     " },
                        if selected {
                            accent_style()
                        } else {
                            muted_style()
                        },
                    ),
                    Span::styled(
                        format!("{:<22}", suggestion.usage),
                        if selected {
                            Style::new()
                                .fg(VOID)
                                .bg(MAGENTA)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::new().fg(CYAN).bg(PANEL)
                        },
                    ),
                    Span::styled(format!("  {}", suggestion.description), muted_style()),
                ])
            }),
    );
    frame.render_widget(Paragraph::new(lines).style(panel_style()), area);
}

fn render_agent(frame: &mut Frame<'_>, area: Rect, model: &KillSilenceViewModel) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let active = model.agent.status != KillSilenceAgentStatus::Disconnected;
    let border_color = if active { MAGENTA } else { DIM };
    let title = if active {
        " CLAUDE//EXTERNAL LINK "
    } else {
        " WITH-AGENTS//STANDBY "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title,
            Style::new().fg(border_color).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(border_color))
        .style(panel_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let project = if model.agent.project.is_empty() {
        "NO SESSION SELECTED"
    } else {
        &model.agent.project
    };
    let session_title = if model.agent.session_title.is_empty() {
        "/with-agents TO LINK"
    } else {
        &model.agent.session_title
    };
    let elapsed = model.agent.elapsed_seconds;
    let mut lines = vec![
        Line::from(vec![
            Span::styled("SESSION  ", muted_style()),
            Span::styled(project, title_style()),
        ]),
        Line::from(vec![
            Span::styled("STATE    ", muted_style()),
            Span::styled(
                agent_status(model.agent.status),
                Style::new()
                    .fg(agent_color(model.agent.status))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("ELAPSED  ", muted_style()),
            Span::styled(
                format!("{:02}:{:02}", elapsed / 60, elapsed % 60),
                Style::new().fg(TEXT),
            ),
        ]),
        Line::styled(session_title, muted_style()),
    ];
    let completion_visible = model.agent.status == KillSilenceAgentStatus::Complete
        && completion_banner_visible(model.agent.completion_elapsed_ms);
    if completion_visible {
        lines.push(Line::styled(
            " THE AGENT'S WORK IS COMPLETE!! ",
            Style::new().fg(VOID).bg(GREEN).add_modifier(Modifier::BOLD),
        ));
    } else if model.agent.status == KillSilenceAgentStatus::Complete {
        lines.push(Line::raw(""));
    } else if active {
        lines.push(Line::styled(
            " WATCHING THE EXTERNAL TERMINAL",
            muted_style(),
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .style(panel_style()),
        inner,
    );
}

fn render_overlay(frame: &mut Frame<'_>, overlay: &KillSilenceOverlay) {
    let area = centered_rect(frame.area(), 76, 72);
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(panel_style()), area);
    let items = if overlay.items.is_empty() {
        vec![ListItem::new(Line::styled(
            "NO SIGNALS FOUND",
            muted_style(),
        ))]
    } else {
        overlay
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {:02} ", index + 1), accent_style()),
                    Span::styled(item, Style::new().fg(TEXT)),
                ]))
            })
            .collect()
    };
    let footer = if overlay.footer.is_empty() {
        " ↑↓ SELECT · ENTER OPEN · ESC CLOSE "
    } else {
        &overlay.footer
    };
    let list = List::new(items)
        .block(
            Block::bordered()
                .title(Span::styled(format!(" {} ", overlay.title), accent_style()))
                .title_bottom(Line::styled(footer, muted_style()).alignment(Alignment::Center))
                .border_style(Style::new().fg(MAGENTA))
                .style(panel_style()),
        )
        .highlight_symbol("▸ ")
        .highlight_style(
            Style::new()
                .fg(VOID)
                .bg(MAGENTA)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default().with_selected(Some(
        overlay.selected.min(overlay.items.len().saturating_sub(1)),
    ));
    frame.render_stateful_widget(list, area, &mut state);
}

#[must_use]
pub fn signal_heights(width: u16) -> Vec<u8> {
    signal_heights_at(width, 0)
}

#[must_use]
pub fn signal_heights_at(width: u16, phase: usize) -> Vec<u8> {
    let width = usize::from(width);
    if width == 0 {
        return Vec::new();
    }
    (0..width)
        .map(|column| {
            let source = (column * SIGNAL_PRESET.len() / width + phase) % SIGNAL_PRESET.len();
            SIGNAL_PRESET[source.min(SIGNAL_PRESET.len() - 1)]
        })
        .collect()
}

fn completion_banner_visible(elapsed_ms: Option<u64>) -> bool {
    let Some(elapsed_ms) = elapsed_ms else {
        return true;
    };
    let blinking_for = AGENT_BLINK_HALF_PERIOD_MS * 2 * AGENT_BLINK_COUNT;
    elapsed_ms >= blinking_for || (elapsed_ms / AGENT_BLINK_HALF_PERIOD_MS).is_multiple_of(2)
}

fn playback_ratio(model: &KillSilenceViewModel) -> f64 {
    let duration = model.track.as_ref().map_or(0, |track| track.duration_ms);
    if duration == 0 {
        0.0
    } else {
        (model.progress_ms as f64 / duration as f64).clamp(0.0, 1.0)
    }
}

fn format_time(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

const fn agent_status(status: KillSilenceAgentStatus) -> &'static str {
    match status {
        KillSilenceAgentStatus::Disconnected => "DISCONNECTED",
        KillSilenceAgentStatus::Armed => "ARMED",
        KillSilenceAgentStatus::Working => "WORKING · SOUNDTRACK ACTIVE",
        KillSilenceAgentStatus::Complete => "COMPLETE · SOUNDTRACK HELD",
        KillSilenceAgentStatus::Interrupted => "INTERRUPTED",
        KillSilenceAgentStatus::Error => "LINK ERROR",
    }
}

const fn agent_color(status: KillSilenceAgentStatus) -> Color {
    match status {
        KillSilenceAgentStatus::Working => CYAN,
        KillSilenceAgentStatus::Complete => GREEN,
        KillSilenceAgentStatus::Armed => MAGENTA,
        KillSilenceAgentStatus::Interrupted | KillSilenceAgentStatus::Error => AMBER,
        KillSilenceAgentStatus::Disconnected => MUTED,
    }
}

fn terminal_color(color: Color, mode: ArtworkColorMode) -> Color {
    let true_color = match mode {
        ArtworkColorMode::TrueColor => true,
        ArtworkColorMode::Ansi256 => false,
        ArtworkColorMode::Auto => truecolor_supported(),
    };
    match color {
        Color::Rgb(red, green, blue) if !true_color => {
            Color::Indexed(rgb_to_xterm256(red, green, blue))
        }
        _ => color,
    }
}

fn truecolor_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        std::env::var("COLORTERM").is_ok_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("truecolor") || value.contains("24bit")
        })
    })
}

const fn rgb_to_xterm256(red: u8, green: u8, blue: u8) -> u8 {
    const fn channel(value: u8) -> u8 {
        ((value as u16 * 5 + 127) / 255) as u8
    }
    16 + 36 * channel(red) + 6 * channel(green) + channel(blue)
}

const fn panel_style() -> Style {
    Style::new().fg(TEXT).bg(PANEL)
}

const fn title_style() -> Style {
    Style::new().fg(CYAN).add_modifier(Modifier::BOLD)
}

const fn accent_style() -> Style {
    Style::new().fg(MAGENTA).add_modifier(Modifier::BOLD)
}

const fn muted_style() -> Style {
    Style::new().fg(MUTED).bg(PANEL)
}

fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, buffer::Buffer, style::Color, Terminal};

    use super::{
        completion_banner_visible, render_kill_silence, render_kill_silence_boot,
        render_kill_silence_home, signal_heights_at, terminal_color, ArtworkColorMode,
        KillSilenceAgentStatus, KillSilenceCommandSuggestion, KillSilenceOverlay, KillSilenceTrack,
        KillSilenceViewModel,
    };

    fn playing_model() -> KillSilenceViewModel {
        KillSilenceViewModel {
            authenticated: true,
            account_label: "signal-user".into(),
            status_line: "SIGNAL TRANSMISSION IN PROGRESS".into(),
            track: Some(KillSilenceTrack {
                title: "Last Night on Earth".into(),
                artists: "Green Day".into(),
                album: "21st Century Breakdown".into(),
                duration_ms: 236_000,
            }),
            progress_ms: 58_000,
            is_playing: true,
            ..KillSilenceViewModel::default()
        }
    }

    fn screen(buffer: &Buffer) -> String {
        let area = buffer.area;
        (area.top()..area.bottom())
            .map(|y| {
                let mut row = String::new();
                for x in area.left()..area.right() {
                    row.push_str(buffer[(x, y)].symbol());
                }
                row
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn find(buffer: &Buffer, needle: &str) -> Option<(usize, u16)> {
        let area = buffer.area;
        for y in area.top()..area.bottom() {
            let row = (area.left()..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>();
            if let Some(x) = row.find(needle) {
                return Some((x, y));
            }
        }
        None
    }

    #[test]
    fn eighty_by_twenty_four_keeps_all_primary_regions() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_kill_silence(frame, &playing_model(), None))
            .unwrap();
        let view = screen(terminal.backend().buffer());
        assert!(view.contains("KILL//SILENCE"));
        assert!(view.contains("LAST NIGHT ON EARTH"));
        assert!(view.contains("LIVE SIGNAL WAVEFORM"));
        assert!(view.contains("WITH-AGENTS//STANDBY"));
        assert!(view.contains("ks://"));
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .all(|cell| cell.bg != Color::Reset),
            "every cell must have an explicit dark background"
        );
    }

    #[test]
    fn wide_layout_places_external_agent_at_the_right() {
        let mut model = playing_model();
        model.agent.status = KillSilenceAgentStatus::Working;
        model.agent.project = "kill-silence".into();
        let backend = TestBackend::new(160, 36);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_kill_silence(frame, &model, None))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let player = find(buffer, "KILL//SILENCE").unwrap();
        let agent = find(buffer, "CLAUDE//EXTERNAL LINK").unwrap();
        assert!(agent.0 > player.0);
        assert!(agent.1 <= player.1 + 1);
    }

    #[test]
    fn overlay_and_completion_signal_are_visible() {
        let mut model = playing_model();
        model.agent.status = KillSilenceAgentStatus::Complete;
        model.frame_tick = 1;
        let backend = TestBackend::new(160, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_kill_silence(frame, &model, None))
            .unwrap();
        assert!(screen(terminal.backend().buffer()).contains("THE AGENT'S WORK IS COMPLETE!!"));

        model.overlay = Some(KillSilenceOverlay {
            title: "SONG ARCHIVE".into(),
            items: vec!["Track one".into(), "Track two".into()],
            ..KillSilenceOverlay::default()
        });
        terminal
            .draw(|frame| render_kill_silence(frame, &model, None))
            .unwrap();
        let view = screen(terminal.backend().buffer());
        assert!(view.contains("SONG ARCHIVE"));
        assert!(view.contains("Track one"));
    }

    #[test]
    fn boot_and_terminal_app_color_fallback_are_stable() {
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_kill_silence_boot(frame, 2))
            .unwrap();
        let view = screen(terminal.backend().buffer());
        assert!(view.contains("KILL//SILENCE"));
        assert!(view.contains("TRANSMISSION BEGINS WHERE SILENCE ENDS"));
        assert_eq!(
            terminal_color(Color::Rgb(255, 0, 0), ArtworkColorMode::Ansi256),
            Color::Indexed(196)
        );
        assert_eq!(
            terminal_color(Color::Rgb(12, 34, 56), ArtworkColorMode::TrueColor),
            Color::Rgb(12, 34, 56)
        );

        // Exercise every status as part of the public integration contract.
        for status in [
            KillSilenceAgentStatus::Disconnected,
            KillSilenceAgentStatus::Armed,
            KillSilenceAgentStatus::Working,
            KillSilenceAgentStatus::Complete,
            KillSilenceAgentStatus::Interrupted,
            KillSilenceAgentStatus::Error,
        ] {
            assert!(!super::agent_status(status).is_empty());
        }
    }

    #[test]
    fn home_screen_keeps_commands_and_agent_panel_visible() {
        let mut model = playing_model();
        model.command_line = "/pla".into();
        model.command_suggestions = vec![
            KillSilenceCommandSuggestion {
                usage: "/play".into(),
                description: "resume playback".into(),
            },
            KillSilenceCommandSuggestion {
                usage: "/player".into(),
                description: "show the playing track".into(),
            },
        ];
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_kill_silence_home(frame, &model))
            .unwrap();
        let view = screen(terminal.backend().buffer());
        assert!(view.contains("KILL//SILENCE"));
        assert!(view.contains("/play"));
        assert!(view.contains("/player"));
        assert!(view.contains("WITH-AGENTS//STANDBY"));
    }

    #[test]
    fn preset_waveform_scrolls_and_completion_becomes_solid_after_ten_blinks() {
        assert_ne!(signal_heights_at(40, 0), signal_heights_at(40, 3));
        assert!(completion_banner_visible(Some(0)));
        assert!(!completion_banner_visible(Some(300)));
        assert!(completion_banner_visible(Some(6_000)));
        assert!(completion_banner_visible(Some(60_000)));
    }
}
