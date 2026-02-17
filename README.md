# tui-player

A terminal music player written in Rust.

## Features

- Plays MP3, FLAC, OGG, WAV, and AAC files
- 3 visualizer modes: oscilloscope, vectorscope, spectroscope (braille Unicode)
- Lyrics fetching from lyrics.ovh and Genius
- Album art display (half-block rendering)
- Mouse support (click to seek, adjust volume, toggle lyrics, scroll)
- File browser with tree navigation (press `f`) — accepts directories as input
- Persistent settings (volume, visualizer mode, lyrics visibility)
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
| `j` / `k` | Scroll lyrics |
| `q` / `Ctrl+C` | Quit |

## Configuration

Settings are persisted in `~/.config/tui-player/`:

- `volume` — playback volume (0.0 - 2.0)
- `vis_mode` — visualizer mode (oscilloscope, vectorscope, spectroscope)
- `lyrics_visible` — lyrics panel visibility (true/false)
