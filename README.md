<h1 align="center">KILL//SILENCE</h1>

<p align="center"><strong>English</strong> · <a href="./README.ko.md">한국어</a></p>

<p align="center"><strong>Worried that an agent might take your job? Great. Let it write the code while we listen to music.</strong></p>
<p align="center"><em>The agent cooks; you supervise the vibes. ♪</em></p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-101018?style=flat-square&logo=rust&logoColor=35e5e5" alt="Rust">
  <img src="https://img.shields.io/badge/Spotify-101018?style=flat-square&logo=spotify&logoColor=66ff66" alt="Spotify">
  <img src="https://img.shields.io/badge/Claude%20Code-101018?style=flat-square&logo=anthropic&logoColor=ffd166" alt="Claude Code">
  <img src="https://img.shields.io/badge/Ratatui-101018?style=flat-square&color=ff43cf" alt="Ratatui">
</p>

## Why `kill-silence`?

What are we supposed to do while an agent writes code? Read every log line with professional concentration? Of course not. We click in another prompt, stare into the middle distance, then switch terminals to open a music player. Even that terminal switch feels like unpaid labor.

So **KILL//SILENCE** simply stays in the terminal. It controls Spotify, crushes album covers into terminal pixels, and streams a suspiciously convincing waveform. When Claude Code starts a real turn, the music plays. When the turn ends, the music stops. That is when we look up and return to work. It is less of a productivity tool and more of a very cute warning light for pretending to be productive.

<p align="center">
  <img src="./docs/screenshots/01-main.png" alt="KILL//SILENCE main terminal screen" width="100%">
</p>

## Install in 30 seconds

You need Rust, Spotify Premium, and one active Spotify Desktop or Connect device. Claude Code is only required for `/with-agents`.

```bash
git clone https://github.com/TaewoooPark/kill-silence.git
cd kill-silence
cargo install --path spotify_player --locked --force

# now available from any directory
kill-silence
```

The first launch opens a Spotify approval page in your browser. Approve it once and return to the terminal. **You do not need to enter a Spotify username, Client ID, Client Secret, Developer Dashboard app, or redirect URI.**

If nothing plays, open Spotify Desktop once to create an active Connect device, then choose it inside KILL//SILENCE:

```text
/spotify device
```

Only use the following command when you deliberately want to renew authorization:

```bash
kill-silence authenticate
```

## The entire workflow

```text
# 1. Pick some music
/song

# 2. Select a track or playlist with Enter
#    Music now works even when the agent does not

# 3. If Claude Code is already open, bind its session
/with-agents

# 4. A real Claude turn starts the music
#    The end of the turn stops it
#    That is our cue to resume having a job
```

Type `/` in the command console and an indexed command list appears underneath. Pick an entry with `↑` / `↓`, complete it with `Tab` or `→`, then press `Enter`. You can also type the whole command yourself if you miss manual labor.

## Command dictionary

| Category | Command | What it does |
|---|---|---|
| Find music | `/song` | Opens saved tracks and playlists. Selecting one starts playback. |
|  | `/search <query>` | Searches Spotify for tracks. |
|  | `/queue` | Opens the current Spotify queue. |
| Playback | `/play` | Resumes the paused track. |
|  | `/stop` | Pauses at the current position. |
|  | `/replay` | Seeks to the beginning and plays again. |
|  | `/next` / `/prev` | Moves to the next or previous track. |
|  | `/volume 1..10` | Sets the volume on the active Spotify device. |
| Library | `/like` | Saves the current track to your Spotify library. |
| Device | `/spotify device` | Chooses the Spotify Connect output device. |
| Agent | `/with-agents` | Selects a real Claude Code session and watches its work state. |
| Screen | `/home` | Opens the KILL//SILENCE title screen without interrupting music. |
|  | `/player` | Returns to the album art, waveform, and progress view. |
| Other | `/help` | Opens the command archive. |
|  | `/quit` | Restores the terminal and exits politely. |

### Your hands are already on the keyboard

| Key | What it does |
|---|---|
| `F1` | Opens the home screen. Music keeps playing. |
| `F2` | Opens the player screen. Music keeps playing. |
| `↑` / `↓` | Selects a command suggestion or list item. |
| `Tab` / `→` | Completes the selected command. |
| `Enter` | Runs a command or opens the selected item. |
| `Esc` | Clears the input or closes a modal. |
| `j` / `k` | Moves through an open modal list. |
| `Ctrl-C` | Leaves quietly. |

## `/with-agents`: a work notification made of music

KILL//SILENCE does not create another AI chat, inject prompts into Claude, or copy responses into its own panel. It only watches an existing Claude Code session in read-only mode.

1. Run `/with-agents` and choose a session.
2. Give Claude work in its real terminal as usual.
3. Spotify starts when the turn starts.
4. Spotify pauses and the completion signal appears when the turn completes or is interrupted.

Click prompts in the left terminal. Watch album art in the right terminal. Prepare emotionally in whichever terminal has more cyan. Prompt and response bodies are never rendered or modified by KILL//SILENCE.

## Field recordings

| 01 · pick a track | 02 · the agent works |
|---|---|
| ![Spotify player archive](./docs/screenshots/02-player.png) | ![Claude turn active](./docs/screenshots/03-agent-working.png) |

| 03 · return when the music stops | 04 · pick a session |
|---|---|
| ![Agent work complete](./docs/screenshots/04-agent-complete.png) | ![Claude session picker](./docs/screenshots/05-claude-session-picker.png) |

| 05 · player beside Claude | 06 · home beside Claude |
|---|---|
| ![Spotify player running beside Claude](./docs/screenshots/06-player-running.png) | ![KILL//SILENCE home beside Claude Code](./docs/screenshots/07-claude-home.png) |

## Development

```bash
cargo test -p kill-silence
cargo clippy -p kill-silence --all-targets
cargo fmt --all --check
```

Configuration and caches live at `~/.config/kill-silence` and `~/.cache/kill-silence`. Run `kill-silence --help` for the remaining advanced CLI options.

MIT © 2026 Taewoo Park and spotify-player contributors.
