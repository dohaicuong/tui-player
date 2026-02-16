use std::{
    collections::VecDeque,
    env, fs, io,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use symphonia::core::{
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
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

fn probe_duration(path: &PathBuf) -> Option<Duration> {
    let file = fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;

    let reader = probed.format;
    let track = reader.default_track()?;
    let time_base = track.codec_params.time_base?;
    let n_frames = track.codec_params.n_frames?;
    let time = time_base.calc_time(n_frames);

    Some(Duration::from_secs_f64(time.seconds as f64 + time.frac))
}

impl App {
    fn new(path: &PathBuf) -> Self {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".into());

        let total_duration = probe_duration(path);

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

    fn is_finished(&self) -> bool {
        self.sink.empty()
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: tui <music-file>");
        eprintln!();
        eprintln!("For external visualization, run in another terminal:");
        eprintln!("  scope-tui file {PIPE_PATH}");
        std::process::exit(1);
    }
    let path = PathBuf::from(&args[1]);
    if !path.exists() {
        eprintln!("File not found: {}", path.display());
        std::process::exit(1);
    }

    create_pipe();

    let mut terminal = ratatui::init();
    let mut app = App::new(&path);
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    remove_pipe();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
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
                        _ => {}
                    }
                }
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

fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
    ])
    .split(frame.area());

    // Now playing
    let status = if app.paused { "Paused" } else { "Playing" };
    let title = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {status} "), Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(&app.file_name, Style::default().fg(Color::White)),
    ]))
    .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Now Playing "));
    frame.render_widget(title, chunks[0]);

    // Visualizer
    let vis_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(app.vis_mode.label());
    match app.vis_mode {
        VisMode::Oscilloscope => {
            let w = OscilloscopeWidget::new(&app.samples, app.channels).block(vis_block);
            frame.render_widget(w, chunks[1]);
        }
        VisMode::Vectorscope => {
            let w = VectorscopeWidget::new(&app.samples, app.channels).block(vis_block);
            frame.render_widget(w, chunks[1]);
        }
        VisMode::Spectroscope => {
            let w = SpectroscopeWidget::new(&app.samples, app.channels).block(vis_block);
            frame.render_widget(w, chunks[1]);
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
    let help = Paragraph::new(Line::from(vec![
        Span::styled(" Space ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Play/Pause  "),
        Span::styled(" ←/→ ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Seek ±5s  "),
        Span::styled(" ↑/↓ ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Volume  "),
        Span::styled(" v ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Vis Mode  "),
        Span::styled(" q ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Quit"),
    ]))
    .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Controls "));
    frame.render_widget(help, chunks[4]);
}
