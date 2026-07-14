<pre align="center">
██╗  ██╗ ██╗ ██╗     ██╗          ██╗ ██╗     ███████╗ ██╗ ██╗     ███████╗ ███╗   ██╗  ██████╗ ███████╗
██║ ██╔╝ ██║ ██║     ██║         ██╔╝██╔╝     ██╔════╝ ██║ ██║     ██╔════╝ ████╗  ██║ ██╔════╝ ██╔════╝
█████╔╝  ██║ ██║     ██║        ██╔╝██╔╝      ███████╗ ██║ ██║     █████╗   ██╔██╗ ██║ ██║      █████╗
██╔═██╗  ██║ ██║     ██║       ██╔╝██╔╝       ╚════██║ ██║ ██║     ██╔══╝   ██║╚██╗██║ ██║      ██╔══╝
██║  ██╗ ██║ ███████╗███████╗ ██╔╝██╔╝        ███████║ ██║ ███████╗███████╗ ██║ ╚████║ ╚██████╗ ███████╗
╚═╝  ╚═╝ ╚═╝ ╚══════╝╚══════╝ ╚═╝ ╚═╝         ╚══════╝ ╚═╝ ╚══════╝╚══════╝ ╚═╝  ╚═══╝  ╚═════╝ ╚══════╝
</pre>

<p align="center"><strong>Worried an agent will replace your job? Let it code. You have a song to hear.</strong></p>
<p align="center"><em>The agent cooks; you supervise the vibes. ♪</em></p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-101018?style=flat-square&logo=rust&logoColor=35e5e5" alt="Rust">
  <img src="https://img.shields.io/badge/Spotify-101018?style=flat-square&logo=spotify&logoColor=66ff66" alt="Spotify">
  <img src="https://img.shields.io/badge/Claude%20Code-101018?style=flat-square&logo=anthropic&logoColor=ffd166" alt="Claude Code">
  <img src="https://img.shields.io/badge/Ratatui-101018?style=flat-square&color=ff43cf" alt="Ratatui">
</p>

KILL//SILENCE is a cyan-and-magenta Spotify terminal for the most important part of agentic coding: the bit where the agent is typing and you are, very responsibly, listening to music. It keeps the playful terminal from the original local demo—colour album art, a dense vertical signal preset, slash commands, and responsive agent panels—while replacing local files with Spotify library, search, queue, and Connect playback.

Its tiny trick is `/with-agents`: choose a real external Claude Code session, then go back to your actual terminal and let the little soundtrack assistant keep watch. When a turn starts, the music resumes. When Claude finishes, Spotify pauses and **THE AGENT'S WORK IS COMPLETE!!** blinks ten times, then stays lit in the panel—your cue to pretend you were watching the whole time.

No embedded prompt box. No copied responses. No new AI workspace. Just a cheerful audio sidecar for the terminal you already use while your replacement writes the boilerplate.

<p align="center">
  <img src="./docs/screenshots/01-main.png" alt="KILL//SILENCE main terminal screen" width="100%">
</p>

## What survived the rebuild

- Full-colour half-block album cover art with an ANSI-256 fallback for Terminal.app
- A fixed, dense vertical waveform preset that scrolls with playback—decorative on purpose, not fake analysis
- A visually separated command console and indexed slash suggestions with arrow-key selection and Tab completion
- `/home` and `/player` switch views without touching the active Spotify playback
- `/song`, Spotify search, queue, like, playback, volume, and Connect-device commands
- `/with-agents` read-only Claude Code session discovery and lifecycle tracking
- Wide terminals place the agent panel on the right; narrow terminals place it below
- Claude turn start resumes music; completion or interruption pauses it
- Spotify advances normally through the selected playlist or track list
- The original retro-futuristic boot screen and blinking completion signal

## Install

You need Rust, a Spotify Premium account, and an active Spotify Desktop/Connect device.

```bash
git clone https://github.com/TaewoooPark/kill-silence.git
cd kill-silence
cargo install --path spotify_player --locked --force

# available from any directory
kill-silence
```

On first launch, your browser opens Spotify's approval page. Approve it once and return to the terminal. KILL//SILENCE uses the bundled extended-quota client inherited from `spotify-player`; you do **not** enter a Spotify username, Client ID, Client Secret, Developer Dashboard redirect URI, or create an app.

To deliberately renew the authorization later:

```bash
kill-silence authenticate
```

KILL//SILENCE controls Spotify Desktop or another Connect player rather than creating a second local Spotify device. Open Spotify on the target device once if Spotify reports that no active device exists.

## Commands

| Command | Signal |
|---|---|
| `/song` | Open saved tracks and playlists |
| `/search <query>` | Search Spotify tracks |
| `/spotify device` | Choose a Spotify Connect device |
| `/queue` | Show the current Spotify queue |
| `/like` | Save the current track |
| `/with-agents` | Bind to an external Claude Code session |
| `/play` | Continue playback |
| `/stop` | Pause at the current position |
| `/replay` | Seek to the beginning and play |
| `/next` / `/prev` | Change track |
| `/volume 1..10` | Set the active device volume |
| `/home` | Return to the KILL//SILENCE title without stopping music |
| `/player` | Return to the album-art, waveform, and progress view |
| `/help` | Open the command archive |
| `/quit` | Restore the terminal and exit |

Start typing `/` to open the indexed command list. Use `↑` / `↓` to choose a suggestion, `Tab` or `→` to complete it, and `Enter` to run it. `F1` jumps to the title screen and `F2` returns to the player without changing playback. Modal lists additionally support `j` / `k`; `Esc` closes or clears, and `Ctrl-C` exits.

## Seven little states

| 01 · main signal | 02 · choose a player signal |
|---|---|
| ![KILL//SILENCE main screen](./docs/screenshots/01-main.png) | ![Spotify player archive](./docs/screenshots/02-player.png) |

| 03 · player beside Claude | 04 · home, with Claude next door |
|---|---|
| ![Spotify player running beside Claude](./docs/screenshots/06-player-running.png) | ![KILL//SILENCE home beside Claude Code](./docs/screenshots/07-claude-home.png) |

| 05 · pick a Claude session | 06 · agent working |
|---|---|
| ![Claude session picker](./docs/screenshots/05-claude-session-picker.png) | ![Claude turn active](./docs/screenshots/03-agent-working.png) |

### 07 · the sound drops; your turn

![Agent work complete](./docs/screenshots/04-agent-complete.png)

The screenshots show the Spotify rebuild itself: from the standalone home signal to the player, Claude session binding, active work, and the moment the soundtrack drops to tell you the turn is yours again.

## How `/with-agents` works

1. KILL//SILENCE indexes resumable JSONL sessions below `~/.claude/projects`.
2. Selecting one starts a read-only watcher at the file's current end, so old turns do nothing.
3. A new real Claude Code turn resumes Spotify playback.
4. Claude's completion or interruption record pauses playback. Completion blinks the signal ten times and then leaves it steadily visible.

Prompt and response bodies are never rendered by KILL//SILENCE. The watcher emits lifecycle metadata only and never writes to Claude's files.

## Development

```bash
cargo test -p kill-silence
cargo clippy -p kill-silence --all-targets
cargo fmt --all --check
```

Configuration and caches live at `~/.config/kill-silence` and `~/.cache/kill-silence`. The mature non-interactive Spotify CLI subcommands inherited from upstream remain available through `kill-silence --help`.

## Lineage

KILL//SILENCE is a fork of [aome510/spotify-player](https://github.com/aome510/spotify-player), whose Spotify Web API, OAuth PKCE, caching, Connect control, library/search, and CLI foundations power this rebuild. `spotify-player` itself was inspired by `spotify-tui` and `ncspot`. The KILL//SILENCE TUI, slash-command runtime, fixed signal display, and Claude Code bridge are this fork's layer.

MIT © 2026 Taewoo Park and spotify-player contributors.
