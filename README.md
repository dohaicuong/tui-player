# tui-player

A terminal music player written in Rust.

## Features

- Plays MP3, FLAC, OGG, WAV, and AAC files
- 3 visualizer modes: oscilloscope, vectorscope, spectroscope (braille Unicode)
- Lyrics fetching from lyrics.ovh and Genius
- Album art overlay on visualizer (semi-transparent half-block rendering)
- Mouse support (click to seek, adjust volume, toggle lyrics, scroll, hover tooltips on seek/volume/EQ)
- File browser with tree navigation (press `f`) — accepts directories as input, fuzzy search with `/`
- Shuffle and repeat modes (Off / All / One)
- Gapless playback with pre-buffered next track (or crossfade with `c` key — Off / 2s / 5s / 8s)
- ReplayGain volume normalization (reads track/album gain tags)
- Track position indicator (e.g. "3/15") in now playing panel
- 32-band graphic equalizer with presets (press `e`) — real-time biquad filtering
- Waveform preview on seek bar (progressive background scan, block character rendering)
- Persistent settings (volume, visualizer mode, lyrics visibility, EQ, crossfade)
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
| `n` | Next track |
| `N` | Previous track |
| `s` | Toggle shuffle |
| `r` | Cycle repeat (Off / All / One) |
| `c` | Cycle crossfade (Off / 2s / 5s / 8s) |
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
| Mouse click | Select band |
| Scroll wheel | Adjust hovered band ±1 dB |

## Configuration

Settings are persisted in `~/.config/tui-player/`:

- `volume` — playback volume (0.0 - 2.0)
- `vis_mode` — visualizer mode (oscilloscope, vectorscope, spectroscope)
- `lyrics_visible` — lyrics panel visibility (true/false)
- `eq` — equalizer state (enabled, preset, per-band gains)
- `repeat_mode` — repeat mode (off, all, one)
- `shuffle` — shuffle on/off
- `crossfade` — crossfade duration in seconds (0 = off)
