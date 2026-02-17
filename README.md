# tui-player

A terminal music player written in Rust.

## Features

- Plays MP3, FLAC, OGG, WAV, and AAC files
- 3 visualizer modes: oscilloscope, vectorscope, spectroscope (braille Unicode)
- Lyrics fetching from lyrics.ovh and Genius
- Album art display (half-block rendering)
- Mouse support (click to seek, adjust volume, toggle lyrics, scroll)
- File browser with tree navigation (press `f`) — accepts directories as input
- Auto-advances to the next track when a song finishes
- 32-band graphic equalizer with presets (press `e`) — real-time biquad filtering
- Persistent settings (volume, visualizer mode, lyrics visibility, EQ)
- Optional [scope-tui](https://github.com/alecdotninja/scope-tui) integration via named pipe

## System Dependencies

### Linux

ALSA is required for audio output:

| Distro | Command |
|---|---|
| Arch / CachyOS | `pacman -S alsa-lib` |
| Debian / Ubuntu | `apt install libasound2-dev` |
| Fedora | `dnf install alsa-lib-devel` |

### macOS

No extra dependencies — audio output uses CoreAudio, which is bundled with the OS.

All other dependencies are pure Rust and handled by Cargo.

## Install

```sh
cargo install --path .
```

## Usage

```sh
tui-player <music-file-or-directory>
```

## Keybindings

| Key | Action |
|---|---|
| `Space` | Play / Pause |
| `Left` / `Right` | Seek -/+ 5s |
| `Up` / `Down` | Volume up / down |
| `v` | Cycle visualizer mode |
| `l` | Toggle lyrics panel |
| `f` | Open file browser |
| `e` | Open equalizer |
| `j` / `k` | Scroll lyrics |
| `q` / `Ctrl+C` | Quit |

### Equalizer Controls (when open)

| Key | Action |
|---|---|
| `Left` / `Right` | Select band |
| `Up` / `Down` | Adjust gain ±1 dB |
| `p` | Cycle preset |
| `0` | Reset to flat |
| `s` | Toggle EQ on/off |
| `Esc` / `e` | Close equalizer |

## Configuration

Settings are persisted in `~/.config/tui-player/`:

- `volume` — playback volume (0.0 - 2.0)
- `vis_mode` — visualizer mode (oscilloscope, vectorscope, spectroscope)
- `lyrics_visible` — lyrics panel visibility (true/false)
- `eq` — equalizer state (enabled, preset, per-band gains)
