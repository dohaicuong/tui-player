# tui-player Project Context

## Overview
TUI music player written in Rust. Multi-module architecture (~500 lines in main.rs, ~1250 across modules).

## Tech Stack
- **Rust** (Edition 2024)
- **ratatui** 0.30 — TUI framework
- **crossterm** 0.29 — terminal input/rendering
- **rodio** 0.21 — audio playback
- **symphonia** 0.5 — audio decoding (MP3, FLAC, OGG, WAV, AAC, ISO MP4)
- **rustfft** 6.4 — FFT for spectroscope visualizer
- **image** 0.25 — album art resizing
- **ureq** 3 — HTTP client (lyrics/art fetching)
- **serde_json** 1 — JSON parsing
- **tui-tree-widget** 0.24 — file browser tree widget
- **libc** 0.2 — named pipe creation

## Key File Map
- `src/main.rs` — App struct, PipedSource, event loop (`run()`), playback logic, config I/O, TrackMeta, probe_file(), draw() orchestration, SampleBuf type alias, switch_track()
- `src/file_browser.rs` — File browser overlay: scan_directory(), draw_file_browser(), selected_file(), AUDIO_EXTENSIONS, is_audio_file()
- `src/now_playing.rs` — Now Playing panel: AlbumArtWidget, fetch/spawn_art_fetch, draw_now_playing (vertical art panel), draw_now_playing_bar (horizontal compact bar), ART_ROWS/ART_COLS, ArtPixels type
- `src/visualizer.rs` — VisMode enum, braille constants, OscilloscopeWidget, VectorscopeWidget, SpectroscopeWidget, draw_visualizer()
- `src/lyrics.rs` — LyricsResult, url_encode, html_to_text/decode_entity, fetch_lyrics_ovh, fetch_lyrics_genius, spawn_lyrics_fetchers, draw_lyrics, draw_lyrics_collapsed
- `src/gauge.rs` — RoundedGauge widget (shared by progress and volume)
- `src/progress.rs` — draw_progress(), format_duration()
- `src/volume.rs` — draw_volume()
- `src/controls.rs` — draw_controls(), draw_scope_hint()

## Cross-Module Dependencies
- `gauge.rs` is used by `progress.rs` and `volume.rs` via `crate::gauge::RoundedGauge`
- `visualizer.rs` uses `crate::SampleBuf` (pub type alias in main.rs)
- `now_playing.rs` uses `crate::TrackMeta` (pub struct in main.rs)
- `main.rs` re-exports from modules: `spawn_art_fetch`, `ArtPixels`, `ART_COLS`, `ART_ROWS`, `VisMode`, `spawn_lyrics_fetchers`, `LyricsResult`

## Implemented Features
- File and directory playback (MP3/FLAC/OGG/WAV/AAC/M4A)
- 3 visualizer modes: oscilloscope, vectorscope, spectroscope (braille Unicode)
- Lyrics fetching from lyrics.ovh + Genius web scraping
- Album art from Genius search results (half-block rendering)
- Mouse support (click seek, volume, play/pause, lyrics toggle, scroll)
- File browser tree overlay (f key) — browse directories, select tracks, switch playback
- Keyboard: Space=play/pause, arrows=seek/volume, v=vis mode, l=lyrics, f=file browser, j/k=scroll, q/Ctrl+C=quit
- Persistent config at `~/.config/tui-player/` (volume, vis_mode, lyrics_visible)
- Optional scope-tui integration via named pipe `/tmp/tui-player.pipe`
- Adaptive layout (compact vs vertical left panel when album art loads)

## System Dependencies
Audio output via rodio/cpal:
- **Linux**: requires **ALSA** (`alsa-lib` / `libasound2-dev` / `alsa-lib-devel`)
- **macOS**: uses **CoreAudio** (bundled with the OS, no extra packages needed)

All other crate dependencies (symphonia, rustfft, image, ureq, ratatui, crossterm) are pure Rust.

## Config Paths
- `~/.config/tui-player/volume`
- `~/.config/tui-player/vis_mode`
- `~/.config/tui-player/lyrics_visible`
