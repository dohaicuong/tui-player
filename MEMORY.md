# tui-player Project Context

## Overview
TUI music player written in Rust. Single-file architecture (~1720 lines in `src/main.rs`).

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
- **libc** 0.2 — named pipe creation

## Key File Map
- `src/main.rs` — entire application (monolithic)
  - Lines 40-128: `PipedSource<S>` audio capture wrapper
  - Lines 130-154: `VisMode` enum
  - Lines 159-260: `OscilloscopeWidget`
  - Lines 262-395: `VectorscopeWidget`
  - Lines 397-550: `SpectroscopeWidget`
  - Lines 552-657: `RoundedGauge` widget
  - Lines 659-693: `App` struct
  - Lines 700-740: Config I/O (persistence)
  - Lines 742-939: Lyrics fetching (lyrics.ovh + Genius scraping)
  - Lines 941-1002: Album art fetching + `AlbumArtWidget`
  - Lines 1029-1112: `TrackMeta` + `probe_file()` metadata extraction
  - Lines 1114-1251: `App` impl (playback, seeking, volume)
  - Lines 1253-1296: `main()` entry point
  - Lines 1298-1424: `run()` event loop
  - Lines 1426-1720: `draw()` UI rendering

## Implemented Features
- Single-file playback (MP3/FLAC/OGG/WAV/AAC)
- 3 visualizer modes: oscilloscope, vectorscope, spectroscope (braille Unicode)
- Lyrics fetching from lyrics.ovh + Genius web scraping
- Album art from Genius search results (half-block rendering)
- Mouse support (click seek, volume, play/pause, lyrics toggle, scroll)
- Keyboard: Space=play/pause, arrows=seek/volume, v=vis mode, l=lyrics, j/k=scroll, q=quit
- Persistent config at `~/.config/tui-player/` (volume, vis_mode, lyrics_visible)
- Optional scope-tui integration via named pipe `/tmp/tui-player.pipe`
- Adaptive layout (compact vs vertical left panel when album art loads)

## Config Paths
- `~/.config/tui-player/volume`
- `~/.config/tui-player/vis_mode`
- `~/.config/tui-player/lyrics_visible`
