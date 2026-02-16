use std::{
    collections::VecDeque,
    env, fs, io,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

use symphonia::core::{
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey, Value},
    probe::Hint,
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget},
    DefaultTerminal, Frame,
};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use rustfft::{num_complex::Complex, FftPlanner};

const PIPE_PATH: &str = "/tmp/tui-player.pipe";

type SampleBuf = Arc<Mutex<VecDeque<f32>>>;
const SAMPLE_BUF_SIZE: usize = 8192;
const ART_ROWS: u16 = 16;
const ART_COLS: u16 = ART_ROWS * 2; // 2 cols per row for square aspect

// Source wrapper that writes to pipe and captures samples for visualization
struct PipedSource<S> {
    inner: S,
    pipe: Option<fs::File>,
    pipe_ready: Arc<AtomicBool>,
    samples: SampleBuf,
}

impl<S> PipedSource<S>
where
    S: Source<Item = f32>,
{
    fn new(source: S, pipe_ready: Arc<AtomicBool>, samples: SampleBuf) -> Self {
        PipedSource {
            inner: source,
            pipe: None,
            pipe_ready,
            samples,
        }
    }

    fn ensure_pipe(&mut self) {
        if self.pipe.is_none() && self.pipe_ready.load(Ordering::Relaxed) {
            let file = fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(PIPE_PATH);
            if let Ok(f) = file {
                self.pipe = Some(f);
            }
        }
    }
}

impl<S> Iterator for PipedSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;

        // Write to pipe for external scope-tui
        self.ensure_pipe();
        if let Some(ref mut pipe) = self.pipe {
            let clamped = sample.clamp(-1.0, 1.0);
            let i16_sample = (clamped * 32767.0) as i16;
            if io::Write::write_all(pipe, &i16_sample.to_le_bytes()).is_err() {
                self.pipe = None;
            }
        }

        // Store in ring buffer for built-in visualizer
        if let Ok(mut buf) = self.samples.try_lock() {
            if buf.len() >= SAMPLE_BUF_SIZE {
                buf.pop_front();
            }
            buf.push_back(sample);
        }

        Some(sample)
    }
}

impl<S> Source for PipedSource<S>
where
    S: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)
    }
}

// Visualization modes
#[derive(Clone, Copy, PartialEq)]
enum VisMode {
    Oscilloscope,
    Vectorscope,
    Spectroscope,
}

impl VisMode {
    fn next(self) -> Self {
        match self {
            VisMode::Oscilloscope => VisMode::Vectorscope,
            VisMode::Vectorscope => VisMode::Spectroscope,
            VisMode::Spectroscope => VisMode::Oscilloscope,
        }
    }

    fn label(self) -> &'static str {
        match self {
            VisMode::Oscilloscope => " Oscilloscope ",
            VisMode::Vectorscope => " Vectorscope ",
            VisMode::Spectroscope => " Spectroscope ",
        }
    }
}

// Braille dot positions per character cell (2 wide x 4 tall):
//   col0: bits 0,1,2,6  (top to bottom)
//   col1: bits 3,4,5,7  (top to bottom)
const BRAILLE_BASE: u32 = 0x2800;
const BRAILLE_DOTS: [[u8; 4]; 2] = [
    [0x01, 0x02, 0x04, 0x40], // left column
    [0x08, 0x10, 0x20, 0x80], // right column
];

struct OscilloscopeWidget<'a> {
    samples: &'a SampleBuf,
    channels: u16,
    block: Option<Block<'a>>,
}

impl<'a> OscilloscopeWidget<'a> {
    fn new(samples: &'a SampleBuf, channels: u16) -> Self {
        OscilloscopeWidget {
            samples,
            channels,
            block: None,
        }
    }

    fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl Widget for OscilloscopeWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = if let Some(block) = self.block {
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            area
        };

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let samples: Vec<f32> = if let Ok(s) = self.samples.lock() {
            s.iter().copied().collect()
        } else {
            return;
        };

        if samples.is_empty() {
            return;
        }

        let ch_count = self.channels.max(1) as usize;
        let px_w = inner.width as usize * 2;
        let px_h = inner.height as usize * 4;
        let mid_y = px_h as f32 / 2.0;

        let cols = inner.width as usize;
        let rows = inner.height as usize;
        let mut grid = vec![0u8; cols * rows];

        // Draw center reference line
        let center_py = px_h / 2;
        let center_cy = center_py / 4;
        let center_dy = center_py % 4;
        if center_cy < rows {
            for cx in 0..cols {
                grid[center_cy * cols + cx] |=
                    BRAILLE_DOTS[0][center_dy] | BRAILLE_DOTS[1][center_dy];
            }
        }
        let ref_grid = grid.clone();

        // Plot waveform (left channel)
        let total_mono = samples.len() / ch_count;
        for px_x in 0..px_w {
            let sample_idx = (px_x * total_mono) / px_w;
            let s = samples.get(sample_idx * ch_count).copied().unwrap_or(0.0);
            let py = ((1.0 - s.clamp(-1.0, 1.0)) * mid_y).min(px_h as f32 - 1.0) as usize;

            let cx = px_x / 2;
            let cy = py / 4;
            let dx = px_x % 2;
            let dy = py % 4;

            if cx < cols && cy < rows {
                grid[cy * cols + cx] |= BRAILLE_DOTS[dx][dy];
            }
        }

        for cy in 0..rows {
            for cx in 0..cols {
                let dots = grid[cy * cols + cx];
                let ch = char::from_u32(BRAILLE_BASE + dots as u32).unwrap_or(' ');
                let x = inner.x + cx as u16;
                let y = inner.y + cy as u16;
                let has_wave = (dots & !ref_grid[cy * cols + cx]) != 0;
                let color = if has_wave { Color::Green } else { Color::DarkGray };
                buf[(x, y)].set_char(ch).set_fg(color);
            }
        }
    }
}

struct VectorscopeWidget<'a> {
    samples: &'a SampleBuf,
    channels: u16,
    block: Option<Block<'a>>,
}

impl<'a> VectorscopeWidget<'a> {
    fn new(samples: &'a SampleBuf, channels: u16) -> Self {
        VectorscopeWidget {
            samples,
            channels,
            block: None,
        }
    }

    fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl Widget for VectorscopeWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = if let Some(block) = self.block {
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            area
        };

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let samples: Vec<f32> = if let Ok(s) = self.samples.lock() {
            s.iter().copied().collect()
        } else {
            return;
        };

        if samples.is_empty() {
            return;
        }

        let ch_count = self.channels.max(1) as usize;
        let px_w = inner.width as usize * 2;
        let px_h = inner.height as usize * 4;
        let mid_x = px_w as f32 / 2.0;
        let mid_y = px_h as f32 / 2.0;
        // Use the smaller dimension so the plot is square
        let radius = mid_x.min(mid_y);

        let cols = inner.width as usize;
        let rows = inner.height as usize;
        let mut grid = vec![0u8; cols * rows];

        // Draw crosshair reference lines (dimmed)
        // Vertical center line
        let center_px_x = px_w / 2;
        for py in 0..px_h {
            let cx = center_px_x / 2;
            let dx = center_px_x % 2;
            let cy = py / 4;
            let dy = py % 4;
            if cx < cols && cy < rows {
                grid[cy * cols + cx] |= BRAILLE_DOTS[dx][dy];
            }
        }
        // Horizontal center line
        let center_py = px_h / 2;
        for px_x in 0..px_w {
            let cx = px_x / 2;
            let dx = px_x % 2;
            let cy = center_py / 4;
            let dy = center_py % 4;
            if cx < cols && cy < rows {
                grid[cy * cols + cx] |= BRAILLE_DOTS[dx][dy];
            }
        }

        // Track which cells have crosshair bits for coloring
        let ref_grid = grid.clone();

        // Plot L/R sample pairs using mid/side rotation:
        //   X = (L - R) * 0.707  (side — stereo spread)
        //   Y = (L + R) * 0.707  (mid — mono content)
        // Mono = vertical line, stereo = wider spread
        let num_frames = samples.len() / ch_count;
        for i in 0..num_frames {
            let left = samples[i * ch_count].clamp(-1.0, 1.0);
            let right = if ch_count >= 2 {
                samples[i * ch_count + 1].clamp(-1.0, 1.0)
            } else {
                left
            };

            let side = (left - right) * 0.707;
            let mid = (left + right) * 0.707;

            let px_x = (mid_x + side * radius).clamp(0.0, px_w as f32 - 1.0) as usize;
            let py = (mid_y - mid * radius).clamp(0.0, px_h as f32 - 1.0) as usize;

            let cx = px_x / 2;
            let cy = py / 4;
            let dx = px_x % 2;
            let dy = py % 4;

            if cx < cols && cy < rows {
                grid[cy * cols + cx] |= BRAILLE_DOTS[dx][dy];
            }
        }

        // Render to buffer
        for cy in 0..rows {
            for cx in 0..cols {
                let dots = grid[cy * cols + cx];
                let ch = char::from_u32(BRAILLE_BASE + dots as u32).unwrap_or(' ');
                let x = inner.x + cx as u16;
                let y = inner.y + cy as u16;

                let has_wave = (dots & !ref_grid[cy * cols + cx]) != 0;

                let color = if has_wave {
                    Color::Green
                } else {
                    Color::DarkGray
                };

                buf[(x, y)].set_char(ch).set_fg(color);
            }
        }
    }
}

struct SpectroscopeWidget<'a> {
    samples: &'a SampleBuf,
    channels: u16,
    block: Option<Block<'a>>,
}

impl<'a> SpectroscopeWidget<'a> {
    fn new(samples: &'a SampleBuf, channels: u16) -> Self {
        SpectroscopeWidget {
            samples,
            channels,
            block: None,
        }
    }

    fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl Widget for SpectroscopeWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = if let Some(block) = self.block {
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            area
        };

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let samples: Vec<f32> = if let Ok(s) = self.samples.lock() {
            s.iter().copied().collect()
        } else {
            return;
        };

        if samples.is_empty() {
            return;
        }

        let ch_count = self.channels.max(1) as usize;
        let px_h = inner.height as usize * 4;
        let cols = inner.width as usize;
        let rows = inner.height as usize;

        // Mix down to mono
        let num_frames = samples.len() / ch_count;
        let mut mono: Vec<f32> = Vec::with_capacity(num_frames);
        for i in 0..num_frames {
            let mut sum = 0.0;
            for c in 0..ch_count {
                sum += samples[i * ch_count + c];
            }
            mono.push(sum / ch_count as f32);
        }

        // FFT — use power-of-2 window
        let fft_size = mono.len().next_power_of_two().max(64);
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        let mut fft_input: Vec<Complex<f32>> = Vec::with_capacity(fft_size);
        // Apply Hann window
        let window_len = mono.len().min(fft_size);
        for i in 0..window_len {
            let w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (window_len as f32 - 1.0)).cos());
            fft_input.push(Complex::new(mono[mono.len() - window_len + i] * w, 0.0));
        }
        // Zero-pad remainder
        fft_input.resize(fft_size, Complex::new(0.0, 0.0));

        fft.process(&mut fft_input);

        // Only use first half (positive frequencies)
        let num_bins = fft_size / 2;
        let magnitudes: Vec<f32> = fft_input[..num_bins]
            .iter()
            .map(|c| c.norm() / fft_size as f32)
            .collect();

        // Map bins to columns using logarithmic scale
        // Map frequency bins to screen columns with log scale
        let mut col_mags = vec![0.0f32; cols];
        if num_bins > 1 {
            for col in 0..cols {
                // Log scale: map column to frequency bin
                let frac = col as f32 / cols as f32;
                let bin_f = (num_bins as f32).powf(frac);
                let bin = (bin_f as usize).clamp(1, num_bins - 1);
                // Average nearby bins for smoother result
                let lo = bin.saturating_sub(1);
                let hi = (bin + 1).min(num_bins - 1);
                let mut sum = 0.0;
                let mut count = 0;
                for b in lo..=hi {
                    sum += magnitudes[b];
                    count += 1;
                }
                col_mags[col] = sum / count as f32;
            }
        }

        // Normalize magnitudes
        let max_mag = col_mags.iter().cloned().fold(0.0f32, f32::max).max(0.001);

        // Render using braille — each column bar grows upward from bottom
        let mut grid = vec![0u8; cols * rows];

        for col in 0..cols {
            let height = (col_mags[col] / max_mag * px_h as f32).round() as usize;
            let height = height.min(px_h);

            // Fill from bottom up
            for py in (px_h - height)..px_h {
                let cx = col; // one braille column (left dot) per screen column
                let cy = py / 4;
                let dy = py % 4;
                if cy < rows {
                    grid[cy * cols + cx] |= BRAILLE_DOTS[0][dy] | BRAILLE_DOTS[1][dy];
                }
            }
        }

        for cy in 0..rows {
            for cx in 0..cols {
                let dots = grid[cy * cols + cx];
                let ch = char::from_u32(BRAILLE_BASE + dots as u32).unwrap_or(' ');
                let x = inner.x + cx as u16;
                let y = inner.y + cy as u16;

                let color = if dots != 0 {
                    // Color gradient based on vertical position
                    let frac = cy as f32 / rows as f32;
                    if frac < 0.33 {
                        Color::Red
                    } else if frac < 0.66 {
                        Color::Yellow
                    } else {
                        Color::Green
                    }
                } else {
                    Color::DarkGray
                };

                buf[(x, y)].set_char(ch).set_fg(color);
            }
        }
    }
}

// Rounded gauge widget
struct RoundedGauge<'a> {
    ratio: f64,
    label: String,
    filled_color: Color,
    overflow_at: Option<f64>,
    overflow_color: Color,
    block: Option<Block<'a>>,
}

impl<'a> RoundedGauge<'a> {
    fn new(ratio: f64, label: String, filled_color: Color) -> Self {
        RoundedGauge {
            ratio: ratio.clamp(0.0, 1.0),
            label,
            filled_color,
            overflow_at: None,
            overflow_color: Color::Red,
            block: None,
        }
    }

    fn overflow(mut self, threshold: f64, color: Color) -> Self {
        self.overflow_at = Some(threshold);
        self.overflow_color = color;
        self
    }

    fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl Widget for RoundedGauge<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = if let Some(block) = self.block {
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            area
        };

        if inner.width < 2 || inner.height == 0 {
            return;
        }

        let width = inner.width as usize;
        let filled = (self.ratio * width as f64).round() as usize;
        let overflow_col = self
            .overflow_at
            .map(|t| (t * width as f64).round() as usize)
            .unwrap_or(width);
        let y = inner.y;

        for col in 0..width {
            let x = inner.x + col as u16;
            let fill_color = if col >= overflow_col {
                self.overflow_color
            } else {
                self.filled_color
            };
            let (ch, fg, bg) = if filled == 0 {
                if col == 0 {
                    ('╶', Color::DarkGray, Color::Reset)
                } else if col == width - 1 {
                    ('╴', Color::DarkGray, Color::Reset)
                } else {
                    ('─', Color::DarkGray, Color::Reset)
                }
            } else if col < filled {
                if col == 0 {
                    ('╺', fill_color, Color::Reset)
                } else if col == filled - 1 && filled < width {
                    ('╸', fill_color, Color::Reset)
                } else {
                    ('━', fill_color, Color::Reset)
                }
            } else {
                if col == width - 1 {
                    ('╴', Color::DarkGray, Color::Reset)
                } else {
                    ('─', Color::DarkGray, Color::Reset)
                }
            };

            buf[(x, y)].set_char(ch).set_fg(fg).set_bg(bg);
        }

        let label_len = self.label.len();
        if label_len <= width {
            let start = inner.x + (width - label_len) as u16 / 2;
            for (i, ch) in self.label.chars().enumerate() {
                let x = start + i as u16;
                let col = (x - inner.x) as usize;
                let fg = if col < filled {
                    Color::White
                } else {
                    Color::Gray
                };
                buf[(x, y)].set_char(ch).set_fg(fg).set_bg(Color::Reset);
            }
        }
    }
}

#[derive(Default, Clone)]
struct LayoutRegions {
    now_playing: Rect,
    progress: Rect,
    volume: Rect,
    visualizer: Rect,
    lyrics: Rect,
    lyrics_title: Rect,
}

struct App {
    file_path: PathBuf,
    file_name: String,
    sink: Sink,
    paused: bool,
    volume: f32,
    total_duration: Option<Duration>,
    seek_base: Duration,
    channels: u16,
    pipe_ready: Arc<AtomicBool>,
    samples: SampleBuf,
    stream: OutputStream,
    vis_mode: VisMode,
    show_visualizer: bool,
    meta: TrackMeta,
    regions: LayoutRegions,
    lyrics: Option<LyricsResult>,
    lyrics_scroll: usize,
    lyrics_visible: bool,
    lyrics_loading: bool,
    lyrics_url: String,
    lyrics_rx: Option<mpsc::Receiver<Option<LyricsResult>>>,
    album_art: Option<Vec<Vec<(u8, u8, u8)>>>,
    art_rx: Option<mpsc::Receiver<Vec<Vec<(u8, u8, u8)>>>>,
}

impl App {
    fn position(&self) -> Duration {
        self.seek_base + self.sink.get_pos()
    }
}

fn config_dir() -> PathBuf {
    let home = env::var("HOME").expect("HOME not set");
    PathBuf::from(home).join(".config").join("tui-player")
}

fn load_volume() -> f32 {
    fs::read_to_string(config_dir().join("volume"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1.0)
}

fn save_volume(volume: f32) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("volume"), format!("{volume}"));
}

fn load_vis_mode() -> VisMode {
    fs::read_to_string(config_dir().join("vis_mode"))
        .ok()
        .and_then(|s| match s.trim() {
            "oscilloscope" => Some(VisMode::Oscilloscope),
            "vectorscope" => Some(VisMode::Vectorscope),
            "spectroscope" => Some(VisMode::Spectroscope),
            _ => None,
        })
        .unwrap_or(VisMode::Oscilloscope)
}

fn save_vis_mode(mode: VisMode) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);
    let name = match mode {
        VisMode::Oscilloscope => "oscilloscope",
        VisMode::Vectorscope => "vectorscope",
        VisMode::Spectroscope => "spectroscope",
    };
    let _ = fs::write(dir.join("vis_mode"), name);
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

struct LyricsResult {
    text: String,
    url: String,
    art_url: Option<String>,
}

fn fetch_lyrics_ovh(artist: &str, title: &str) -> Option<LyricsResult> {
    let artist_enc = url_encode(artist);
    let title_enc = url_encode(title);
    let url = format!("https://api.lyrics.ovh/v1/{artist_enc}/{title_enc}");

    let body = ureq::get(&url).call().ok()?.body_mut().read_to_string().ok()?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let text = json.get("lyrics")?.as_str()?.trim().to_string();
    if text.is_empty() { None } else { Some(LyricsResult { text, url, art_url: None }) }
}

fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag_buf = String::new();
    let mut entity_buf = String::new();
    let mut in_entity = false;

    for ch in html.chars() {
        if in_entity {
            entity_buf.push(ch);
            if ch == ';' {
                out.push_str(&decode_entity(&entity_buf));
                entity_buf.clear();
                in_entity = false;
            } else if entity_buf.len() > 10 {
                // Not a real entity, dump it
                out.push_str(&entity_buf);
                entity_buf.clear();
                in_entity = false;
            }
        } else if in_tag {
            tag_buf.push(ch);
            if ch == '>' {
                let lower = tag_buf.to_lowercase();
                if lower.starts_with("<br") {
                    out.push('\n');
                }
                tag_buf.clear();
                in_tag = false;
            }
        } else if ch == '<' {
            in_tag = true;
            tag_buf.clear();
            tag_buf.push(ch);
        } else if ch == '&' {
            in_entity = true;
            entity_buf.clear();
            entity_buf.push(ch);
        } else {
            out.push(ch);
        }
    }
    // Flush leftover
    if in_entity { out.push_str(&entity_buf); }
    if in_tag { out.push_str(&tag_buf); }
    out
}

fn decode_entity(entity: &str) -> String {
    match entity {
        "&amp;" => "&".into(),
        "&lt;" => "<".into(),
        "&gt;" => ">".into(),
        "&quot;" => "\"".into(),
        "&apos;" | "&#x27;" => "'".into(),
        "&nbsp;" => " ".into(),
        _ => {
            // Numeric entities: &#123; or &#x1F;
            let inner = &entity[2..entity.len() - 1]; // strip &# and ;
            if let Some(hex) = inner.strip_prefix('x').or(inner.strip_prefix('X')) {
                u32::from_str_radix(hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| entity.to_string())
            } else if entity.starts_with("&#") {
                inner.parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| entity.to_string())
            } else {
                entity.to_string()
            }
        }
    }
}

fn fetch_lyrics_genius(artist: &str, title: &str) -> Option<LyricsResult> {
    // Search Genius API
    let query = if artist.is_empty() {
        title.to_string()
    } else {
        format!("{artist} {title}")
    };
    let search_url = format!("https://genius.com/api/search?q={}", url_encode(&query));
    let body = ureq::get(&search_url).call().ok()?.body_mut().read_to_string().ok()?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;

    // Get first hit's URL and art
    let hits = json.get("response")?.get("hits")?.as_array()?;
    let result = hits.first()?.get("result")?;
    let song_url = result.get("url")?.as_str()?.to_string();
    let art_url = result.get("song_art_image_thumbnail_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Fetch song page
    let page = ureq::get(&song_url).call().ok()?.body_mut().read_to_string().ok()?;

    // Extract lyrics from <div data-lyrics-container="true"> elements
    let mut lyrics = String::new();
    let marker = "data-lyrics-container=\"true\"";
    let mut search_from = 0;
    while let Some(pos) = page[search_from..].find(marker) {
        let abs = search_from + pos;
        let content_start = match page[abs..].find('>') {
            Some(p) => abs + p + 1,
            None => break,
        };
        // Find matching closing </div> handling nesting
        let mut depth = 1;
        let mut i = content_start;
        while i < page.len() && depth > 0 {
            if page[i..].starts_with("</div>") {
                depth -= 1;
                if depth == 0 { break; }
                i += 6;
            } else if page[i..].starts_with("<div") {
                depth += 1;
                i += 4;
            } else {
                i += page[i..].chars().next().map_or(1, |c| c.len_utf8());
            }
        }
        let raw_html = &page[content_start..i];
        // Convert HTML to plain text: replace <br> with newline, strip tags, decode entities
        let text = html_to_text(raw_html);
        if !text.is_empty() {
            if !lyrics.is_empty() { lyrics.push('\n'); }
            lyrics.push_str(&text);
        }
        search_from = i;
    }

    // Strip Genius metadata prefix from lyrics text
    // Genius often prepends: "1 ContributorSong Title Lyrics[Verse 1]..."
    let mut text = lyrics.trim().to_string();
    if let Some(pos) = text.find(" Lyrics") {
        let after = pos + " Lyrics".len();
        // Only strip if it looks like a metadata prefix (before any lyrics content)
        let before = &text[..pos];
        if before.contains("Contributor") || !before.contains('\n') {
            text = text[after..].trim().to_string();
        }
    }
    if text.is_empty() { None } else { Some(LyricsResult { text, url: song_url, art_url }) }
}

fn spawn_lyrics_fetchers(artist: String, title: String) -> mpsc::Receiver<Option<LyricsResult>> {
    let (tx, rx) = mpsc::channel();

    // Spawn one thread per source — first Some result wins
    let tx1 = tx.clone();
    let a1 = artist.clone();
    let t1 = title.clone();
    thread::spawn(move || {
        let _ = tx1.send(fetch_lyrics_ovh(&a1, &t1));
    });

    let tx2 = tx;
    thread::spawn(move || {
        let _ = tx2.send(fetch_lyrics_genius(&artist, &title));
    });

    rx
}

fn fetch_album_art(url: &str, cols: u16, rows: u16) -> Option<Vec<Vec<(u8, u8, u8)>>> {
    let bytes = ureq::get(url).call().ok()?.body_mut().read_to_vec().ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let px_w = cols as u32;
    let px_h = (rows as u32) * 2; // half-block = 2 pixels per row
    let resized = img.resize_exact(px_w, px_h, image::imageops::FilterType::Lanczos3);
    let rgb = resized.to_rgb8();
    let mut pixels = Vec::with_capacity(px_h as usize);
    for y in 0..px_h {
        let mut row = Vec::with_capacity(px_w as usize);
        for x in 0..px_w {
            let p = rgb.get_pixel(x, y);
            row.push((p[0], p[1], p[2]));
        }
        pixels.push(row);
    }
    Some(pixels)
}

fn spawn_art_fetch(url: String, cols: u16, rows: u16) -> mpsc::Receiver<Vec<Vec<(u8, u8, u8)>>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Some(pixels) = fetch_album_art(&url, cols, rows) {
            let _ = tx.send(pixels);
        }
    });
    rx
}

struct AlbumArtWidget<'a> {
    pixels: &'a [Vec<(u8, u8, u8)>],
}

impl<'a> AlbumArtWidget<'a> {
    fn new(pixels: &'a [Vec<(u8, u8, u8)>]) -> Self {
        AlbumArtWidget { pixels }
    }
}

impl Widget for AlbumArtWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let pixel_rows = self.pixels.len();
        let art_rows = pixel_rows / 2;
        let art_cols = self.pixels.first().map(|r| r.len()).unwrap_or(0);
        let rows = (area.height as usize).min(art_rows);
        let cols = (area.width as usize).min(art_cols);
        for cy in 0..rows {
            let top_y = cy * 2;
            let bot_y = top_y + 1;
            for cx in 0..cols {
                let top = self.pixels[top_y][cx];
                let bot = self.pixels.get(bot_y).map(|r| r[cx]).unwrap_or(top);
                let x = area.x + cx as u16;
                let y = area.y + cy as u16;
                buf[(x, y)]
                    .set_char('▀')
                    .set_fg(Color::Rgb(top.0, top.1, top.2))
                    .set_bg(Color::Rgb(bot.0, bot.1, bot.2));
            }
        }
    }
}

fn load_lyrics_visible() -> bool {
    fs::read_to_string(config_dir().join("lyrics_visible"))
        .ok()
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

fn save_lyrics_visible(visible: bool) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("lyrics_visible"), if visible { "true" } else { "false" });
}

fn create_pipe() {
    let _ = fs::remove_file(PIPE_PATH);
    unsafe {
        let path = std::ffi::CString::new(PIPE_PATH).unwrap();
        libc::mkfifo(path.as_ptr(), 0o644);
    }
}

fn remove_pipe() {
    let _ = fs::remove_file(PIPE_PATH);
}

#[derive(Default)]
struct TrackMeta {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    date: Option<String>,
    genre: Option<String>,
}

struct ProbeInfo {
    duration: Option<Duration>,
    meta: TrackMeta,
}

fn probe_file(path: &PathBuf) -> ProbeInfo {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return ProbeInfo { duration: None, meta: TrackMeta::default() },
    };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut probed = match symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
    {
        Ok(p) => p,
        Err(_) => return ProbeInfo { duration: None, meta: TrackMeta::default() },
    };

    // Extract duration
    let duration = probed.format.default_track().and_then(|track| {
        let time_base = track.codec_params.time_base?;
        let n_frames = track.codec_params.n_frames?;
        let time = time_base.calc_time(n_frames);
        Some(Duration::from_secs_f64(time.seconds as f64 + time.frac))
    });

    // Extract metadata tags
    let mut meta = TrackMeta::default();

    // Collect tags from both the probe metadata and the format metadata
    let mut all_tags: Vec<symphonia::core::meta::Tag> = Vec::new();

    if let Some(rev) = probed.metadata.get().and_then(|mut m| m.skip_to_latest().cloned()) {
        all_tags.extend(rev.tags().iter().cloned());
    }
    if let Some(rev) = probed.format.metadata().skip_to_latest() {
        all_tags.extend(rev.tags().iter().cloned());
    }

    fn tag_string(value: &Value) -> Option<String> {
        match value {
            Value::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    for tag in &all_tags {
        match tag.std_key {
            Some(StandardTagKey::TrackTitle) => {
                if meta.title.is_none() { meta.title = tag_string(&tag.value); }
            }
            Some(StandardTagKey::Artist) | Some(StandardTagKey::AlbumArtist) => {
                if meta.artist.is_none() { meta.artist = tag_string(&tag.value); }
            }
            Some(StandardTagKey::Album) => {
                if meta.album.is_none() { meta.album = tag_string(&tag.value); }
            }
            Some(StandardTagKey::Date) => {
                if meta.date.is_none() { meta.date = tag_string(&tag.value); }
            }
            Some(StandardTagKey::Genre) => {
                if meta.genre.is_none() { meta.genre = tag_string(&tag.value); }
            }
            _ => {}
        }
    }

    ProbeInfo { duration, meta }
}

impl App {
    fn new(path: &PathBuf) -> Self {
        let probe = probe_file(path);
        let file_name = probe.meta.title.clone().unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".into())
        });
        let total_duration = probe.duration;

        let stream = OutputStreamBuilder::from_default_device()
            .expect("failed to find audio device")
            .open_stream_or_fallback()
            .expect("failed to open audio stream");
        let volume = load_volume();
        let sink = Sink::connect_new(stream.mixer());
        sink.set_volume(volume);

        let pipe_ready = Arc::new(AtomicBool::new(true));
        let samples: SampleBuf = Arc::new(Mutex::new(VecDeque::with_capacity(SAMPLE_BUF_SIZE)));

        let file = fs::File::open(path).expect("failed to open file");
        let buf = io::BufReader::new(file);
        let source = Decoder::new(buf).expect("failed to decode audio file");
        let channels = source.channels();
        let piped = PipedSource::new(source, Arc::clone(&pipe_ready), Arc::clone(&samples));
        sink.append(piped);

        // Spawn background lyrics fetch from multiple sources
        let lyrics_artist = probe.meta.artist.clone().unwrap_or_default();
        let lyrics_title = probe.meta.title.clone().unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });
        let has_query = !lyrics_title.is_empty();
        let lyrics_rx = if has_query {
            Some(spawn_lyrics_fetchers(lyrics_artist, lyrics_title))
        } else {
            None
        };

        App {
            file_path: path.clone(),
            file_name,
            sink,
            paused: false,
            volume,
            total_duration,
            seek_base: Duration::ZERO,
            channels,
            pipe_ready,
            samples,
            stream,
            vis_mode: load_vis_mode(),
            show_visualizer: true,
            meta: probe.meta,
            regions: LayoutRegions::default(),
            lyrics: None,
            lyrics_scroll: 0,
            lyrics_visible: load_lyrics_visible(),
            lyrics_loading: has_query,
            lyrics_url: String::new(),
            lyrics_rx,
            album_art: None,
            art_rx: None,
        }
    }

    fn toggle_pause(&mut self) {
        if self.paused {
            self.sink.play();
        } else {
            self.sink.pause();
        }
        self.paused = !self.paused;
    }

    fn volume_up(&mut self) {
        self.volume = ((self.volume * 20.0).round() + 1.0).min(40.0) / 20.0;
        self.sink.set_volume(self.volume);
        save_volume(self.volume);
    }

    fn volume_down(&mut self) {
        self.volume = ((self.volume * 20.0).round() - 1.0).max(0.0) / 20.0;
        self.sink.set_volume(self.volume);
        save_volume(self.volume);
    }

    fn seek(&mut self, offset: i64) {
        let current = self.position();
        let target = if offset >= 0 {
            current + Duration::from_secs(offset as u64)
        } else {
            current.saturating_sub(Duration::from_secs((-offset) as u64))
        };
        self.seek_to(target);
    }

    fn seek_to(&mut self, target: Duration) {
        let clamped = self.total_duration.map(|t| target.min(t)).unwrap_or(target);

        self.sink.stop();
        let new_sink = Sink::connect_new(self.stream.mixer());
        new_sink.set_volume(self.volume);

        let file = fs::File::open(&self.file_path).expect("failed to open file");
        let buf = io::BufReader::new(file);
        let mut source = Decoder::new(buf).expect("failed to decode audio file");
        let _ = source.try_seek(clamped);
        let piped = PipedSource::new(source, Arc::clone(&self.pipe_ready), Arc::clone(&self.samples));
        new_sink.append(piped);

        if self.paused {
            new_sink.pause();
        }

        self.sink = new_sink;
        self.seek_base = clamped;

        if let Ok(mut sbuf) = self.samples.lock() {
            sbuf.clear();
        }
    }

    fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 2.0);
        // Snap to 5% grid
        self.volume = (self.volume * 20.0).round() / 20.0;
        self.sink.set_volume(self.volume);
        save_volume(self.volume);
    }

    fn is_finished(&self) -> bool {
        self.sink.empty()
    }
}

fn has_scope_tui() -> bool {
    std::process::Command::new("which")
        .arg("scope-tui")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let scope_tui_installed = has_scope_tui();
    if args.len() < 2 {
        eprintln!("Usage: tui <music-file>");
        if scope_tui_installed {
            eprintln!();
            eprintln!("For external visualization, run in another terminal:");
            eprintln!("  scope-tui file {PIPE_PATH}");
        }
        std::process::exit(1);
    }
    let path = PathBuf::from(&args[1]);
    if !path.exists() {
        eprintln!("File not found: {}", path.display());
        std::process::exit(1);
    }

    if scope_tui_installed {
        create_pipe();
    }

    crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;
    let mut terminal = ratatui::init();
    let mut app = App::new(&path);
    app.show_visualizer = scope_tui_installed;
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)?;
    if scope_tui_installed {
        remove_pipe();
    }
    result
}

fn hit(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    loop {
        // Poll lyrics results — first Some wins, keep trying until all sources done
        if let Some(ref rx) = app.lyrics_rx {
            loop {
                match rx.try_recv() {
                    Ok(Some(lr)) => {
                        app.lyrics_url = lr.url.clone();
                        // Spawn album art fetch if we got an art URL
                        if let Some(ref art_url) = lr.art_url {
                            app.art_rx = Some(spawn_art_fetch(art_url.clone(), ART_COLS, ART_ROWS));
                        }
                        app.lyrics = Some(lr);
                        app.lyrics_loading = false;
                        app.lyrics_rx = None;
                        break;
                    }
                    Ok(None) => {
                        // This source returned nothing, keep waiting for others
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        // All sources done, none had lyrics
                        app.lyrics_loading = false;
                        app.lyrics_rx = None;
                        break;
                    }
                }
            }
        }

        // Poll album art download
        if let Some(ref rx) = app.art_rx {
            if let Ok(pixels) = rx.try_recv() {
                app.album_art = Some(pixels);
                app.art_rx = None;
            }
        }

        terminal.draw(|f| draw(f, &mut *app))?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char(' ') => app.toggle_pause(),
                    KeyCode::Up => app.volume_up(),
                    KeyCode::Down => app.volume_down(),
                    KeyCode::Right => app.seek(5),
                    KeyCode::Left => app.seek(-5),
                    KeyCode::Char('v') => {
                        app.vis_mode = app.vis_mode.next();
                        save_vis_mode(app.vis_mode);
                    }
                    KeyCode::Char('l') => {
                        app.lyrics_visible = !app.lyrics_visible;
                        save_lyrics_visible(app.lyrics_visible);
                    }
                    KeyCode::Char('j') => app.lyrics_scroll = app.lyrics_scroll.saturating_add(1),
                    KeyCode::Char('k') => app.lyrics_scroll = app.lyrics_scroll.saturating_sub(1),
                    _ => {}
                },
                Event::Mouse(mouse) => {
                    let col = mouse.column;
                    let row = mouse.row;
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            if hit(app.regions.now_playing, col, row) {
                                app.toggle_pause();
                            } else if hit(app.regions.progress, col, row) {
                                if let Some(total) = app.total_duration {
                                    let inner_x = col.saturating_sub(app.regions.progress.x + 1);
                                    let inner_w = app.regions.progress.width.saturating_sub(2);
                                    if inner_w > 0 {
                                        let frac = inner_x as f64 / inner_w as f64;
                                        let target = Duration::from_secs_f64(frac * total.as_secs_f64());
                                        app.seek_to(target);
                                    }
                                }
                            } else if hit(app.regions.volume, col, row) {
                                let inner_x = col.saturating_sub(app.regions.volume.x + 1);
                                let inner_w = app.regions.volume.width.saturating_sub(2);
                                if inner_w > 0 {
                                    let frac = inner_x as f64 / inner_w as f64;
                                    app.set_volume(frac as f32 * 2.0);
                                }
                            } else if (!app.lyrics_visible && hit(app.regions.lyrics, col, row))
                                || (app.lyrics_visible && hit(app.regions.lyrics_title, col, row))
                            {
                                app.lyrics_visible = !app.lyrics_visible;
                                save_lyrics_visible(app.lyrics_visible);
                            } else if hit(app.regions.visualizer, col, row) {
                                app.vis_mode = app.vis_mode.next();
                                save_vis_mode(app.vis_mode);
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            if app.lyrics_visible && hit(app.regions.lyrics, col, row) {
                                app.lyrics_scroll = app.lyrics_scroll.saturating_sub(1);
                            } else {
                                app.volume_up();
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            if app.lyrics_visible && hit(app.regions.lyrics, col, row) {
                                app.lyrics_scroll = app.lyrics_scroll.saturating_add(1);
                            } else {
                                app.volume_down();
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if app.is_finished() && !app.paused {
            break;
        }
    }
    Ok(())
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn draw(frame: &mut Frame, app: &mut App) {
    // Build metadata line
    let mut meta_parts: Vec<String> = Vec::new();
    if let Some(ref artist) = app.meta.artist {
        meta_parts.push(artist.clone());
    }
    if let Some(ref album) = app.meta.album {
        meta_parts.push(album.clone());
    }
    if let Some(ref date) = app.meta.date {
        meta_parts.push(date.clone());
    }
    if let Some(ref genre) = app.meta.genre {
        meta_parts.push(genre.clone());
    }
    let has_meta = !meta_parts.is_empty();
    let has_art = app.album_art.is_some();

    let show_middle = app.show_visualizer || app.lyrics_visible;
    let show_hint = !app.show_visualizer;

    // When album art is available, Now Playing becomes a vertical left panel
    let main_area = if has_art {
        let top_split = Layout::horizontal([
            Constraint::Length(ART_COLS + 2), // art + border
            Constraint::Min(0),
        ]).split(frame.area());

        let np_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Now Playing ");
        let np_inner = np_block.inner(top_split[0]);
        frame.render_widget(np_block, top_split[0]);

        // Build vertical content: status line, blank, art, blank, tags
        let status = if app.paused { "Paused" } else { "Playing" };
        let status_line = Line::from(vec![
            Span::styled(format!(" {status} "), Style::default().fg(Color::Black).bg(Color::Cyan)),
        ]);
        let title_line = Line::from(Span::styled(&app.file_name, Style::default().fg(Color::White)));

        // Render status + title at the top
        let status_rect = Rect::new(np_inner.x, np_inner.y, np_inner.width, 1);
        frame.render_widget(Paragraph::new(status_line), status_rect);
        let title_rect = Rect::new(np_inner.x, np_inner.y + 1, np_inner.width, 1);
        frame.render_widget(Paragraph::new(title_line), title_rect);

        // Render album art below (after 1 blank line)
        let art_y = np_inner.y + 3;
        if let Some(ref pixels) = app.album_art {
            let art_rect = Rect::new(np_inner.x, art_y, ART_COLS.min(np_inner.width), ART_ROWS.min(np_inner.height.saturating_sub(3)));
            frame.render_widget(AlbumArtWidget::new(pixels), art_rect);
        }

        // Render tags below art
        let tags_y = art_y + ART_ROWS + 1;
        if has_meta && tags_y < np_inner.y + np_inner.height {
            let tags_rect = Rect::new(np_inner.x, tags_y, np_inner.width, np_inner.y + np_inner.height - tags_y);
            let mut tag_lines: Vec<Line> = Vec::new();
            if let Some(ref artist) = app.meta.artist {
                tag_lines.push(Line::from(Span::styled(artist.as_str(), Style::default().fg(Color::White))));
            }
            if let Some(ref album) = app.meta.album {
                tag_lines.push(Line::from(Span::styled(album.as_str(), Style::default().fg(Color::DarkGray))));
            }
            if let Some(ref date) = app.meta.date {
                tag_lines.push(Line::from(Span::styled(date.as_str(), Style::default().fg(Color::DarkGray))));
            }
            if let Some(ref genre) = app.meta.genre {
                tag_lines.push(Line::from(Span::styled(genre.as_str(), Style::default().fg(Color::DarkGray))));
            }
            frame.render_widget(Paragraph::new(tag_lines), tags_rect);
        }

        app.regions.now_playing = top_split[0];
        top_split[1]
    } else {
        frame.area()
    };

    // Right-side layout (or full layout when no art)
    let now_playing_height: u16 = if !has_art { if has_meta { 4 } else { 3 } } else { 0 };
    let chunks = Layout::vertical([
        Constraint::Length(now_playing_height),
        if show_middle { Constraint::Min(8) } else { Constraint::Length(0) },
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(if show_hint { 1 } else { 0 }),
    ])
    .split(main_area);

    app.regions.visualizer = chunks[1];
    app.regions.progress = chunks[2];
    app.regions.volume = chunks[3];

    // Now playing (only when no album art — horizontal top bar)
    if !has_art {
        app.regions.now_playing = chunks[0];
        let status = if app.paused { "Paused" } else { "Playing" };
        let mut lines = vec![Line::from(vec![
            Span::styled(format!(" {status} "), Style::default().fg(Color::Black).bg(Color::Cyan)),
            Span::raw("  "),
            Span::styled(&app.file_name, Style::default().fg(Color::White)),
        ])];
        if has_meta {
            lines.push(Line::from(vec![
                Span::raw("         "),
                Span::styled(meta_parts.join("  ·  "), Style::default().fg(Color::DarkGray)),
            ]));
        }
        let title = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Now Playing "));
        frame.render_widget(title, chunks[0]);
    }

    // Determine visualizer and lyrics areas within chunks[1]
    if show_middle {
        let collapsed_w: u16 = 3;
        let (vis_area, lyrics_rect) = if app.show_visualizer && app.lyrics_visible {
            // Both: split 50/50
            let split = Layout::horizontal([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ]).split(chunks[1]);
            (Some(split[0]), split[1])
        } else if app.show_visualizer {
            // Visualizer + collapsed lyrics tab
            let split = Layout::horizontal([
                Constraint::Min(0),
                Constraint::Length(collapsed_w),
            ]).split(chunks[1]);
            (Some(split[0]), split[1])
        } else {
            // No visualizer — lyrics gets full area
            (None, chunks[1])
        };

        if let Some(va) = vis_area {
            app.regions.visualizer = va;
        } else {
            app.regions.visualizer = Rect::default();
        }
        app.regions.lyrics = lyrics_rect;
        app.regions.lyrics_title = Rect::new(lyrics_rect.x, lyrics_rect.y, lyrics_rect.width, 1);

        // Render visualizer
        if let Some(va) = vis_area {
            let vis_block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(app.vis_mode.label());
            match app.vis_mode {
                VisMode::Oscilloscope => {
                    let w = OscilloscopeWidget::new(&app.samples, app.channels).block(vis_block);
                    frame.render_widget(w, va);
                }
                VisMode::Vectorscope => {
                    let w = VectorscopeWidget::new(&app.samples, app.channels).block(vis_block);
                    frame.render_widget(w, va);
                }
                VisMode::Spectroscope => {
                    let w = SpectroscopeWidget::new(&app.samples, app.channels).block(vis_block);
                    frame.render_widget(w, va);
                }
            }
        }

        // Render lyrics panel
        if app.lyrics_visible {
            let lyrics_text = if app.lyrics_loading {
                format!("Loading...\n\n{}", app.lyrics_url)
            } else if let Some(ref lr) = app.lyrics {
                lr.text.clone()
            } else {
                "No lyrics found".to_string()
            };

            let lyrics_lines: Vec<Line> = lyrics_text.lines().map(|l| Line::raw(l)).collect();
            let total_lines = lyrics_lines.len();
            let visible_height = lyrics_rect.height.saturating_sub(2) as usize;
            let max_scroll = total_lines.saturating_sub(visible_height);
            app.lyrics_scroll = app.lyrics_scroll.min(max_scroll);

            let lyrics_title = if app.lyrics_url.is_empty() {
                " Lyrics ".to_string()
            } else {
                format!(" Lyrics - {} ", app.lyrics_url)
            };
            let lyrics_widget = Paragraph::new(lyrics_lines)
                .scroll((app.lyrics_scroll as u16, 0))
                .style(Style::default().fg(Color::White))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(lyrics_title),
                );
            frame.render_widget(lyrics_widget, lyrics_rect);
        } else if app.show_visualizer {
            // Collapsed lyrics tab — only when visualizer is present
            let inner_h = lyrics_rect.height.saturating_sub(2) as usize;
            let label = "Lyrics";
            let pad = inner_h.saturating_sub(label.len()) / 2;
            let mut lines: Vec<Line> = Vec::with_capacity(inner_h);
            for i in 0..inner_h {
                let ch = if i >= pad && i < pad + label.len() {
                    &label[i - pad..i - pad + 1]
                } else {
                    " "
                };
                lines.push(Line::styled(ch, Style::default().fg(Color::DarkGray)));
            }
            let collapsed = Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded),
            );
            frame.render_widget(collapsed, lyrics_rect);
        }
    }

    // Progress
    let elapsed = app.position();
    let progress_label = match app.total_duration {
        Some(total) if !total.is_zero() => {
            format!("{} / {}", format_duration(elapsed), format_duration(total))
        }
        _ => format_duration(elapsed),
    };
    let ratio = app
        .total_duration
        .map(|t| {
            if t.is_zero() {
                0.0
            } else {
                (elapsed.as_secs_f64() / t.as_secs_f64()).min(1.0)
            }
        })
        .unwrap_or(0.0);
    let gauge = RoundedGauge::new(ratio, progress_label, Color::Cyan)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Progress "));
    frame.render_widget(gauge, chunks[2]);

    // Volume
    let vol_pct = (app.volume * 100.0) as u16;
    let vol_ratio = (app.volume / 2.0) as f64;
    let vol_gauge = RoundedGauge::new(vol_ratio, format!("{}%", vol_pct), Color::Green)
        .overflow(0.5, Color::Red)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Volume "));
    frame.render_widget(vol_gauge, chunks[3]);

    // Controls
    let mut help_spans = vec![
        Span::styled(" Space ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Play/Pause  "),
        Span::styled(" ←/→ ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Seek ±5s  "),
        Span::styled(" ↑/↓ ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Volume  "),
    ];
    if app.show_visualizer {
        help_spans.extend([
            Span::styled(" v ", Style::default().fg(Color::Black).bg(Color::Yellow)),
            Span::raw(" Vis Mode  "),
        ]);
    }
    help_spans.extend([
        Span::styled(" l ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Lyrics  "),
    ]);
    help_spans.extend([
        Span::styled(" q ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Quit"),
    ]);
    let help = Paragraph::new(Line::from(help_spans))
    .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Controls "));
    frame.render_widget(help, chunks[4]);

    // Hint to install scope-tui
    if show_hint {
        let hint = Line::from(vec![
            Span::styled(" Run ", Style::default().fg(Color::DarkGray)),
            Span::styled("cargo install scope-tui", Style::default().fg(Color::Yellow)),
            Span::styled(" to enable audio visualizer", Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(Paragraph::new(hint), chunks[5]);
    }
}
