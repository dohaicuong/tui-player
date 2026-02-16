use std::{
    collections::VecDeque,
    env, fs, io,
    path::PathBuf,
    sync::{Arc, Mutex},
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

// Shared ring buffer for audio samples
type SampleBuf = Arc<Mutex<VecDeque<f32>>>;

const SAMPLE_BUF_SIZE: usize = 4096;

// Source wrapper that copies samples to a shared buffer
struct TappedSource<S> {
    inner: S,
    buf: SampleBuf,
}

impl<S> TappedSource<S>
where
    S: Source<Item = f32>,
{
    fn new(source: S, buf: SampleBuf) -> Self {
        TappedSource {
            inner: source,
            buf,
        }
    }
}

impl<S> Iterator for TappedSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;
        if let Ok(mut buf) = self.buf.try_lock() {
            if buf.len() >= SAMPLE_BUF_SIZE {
                buf.pop_front();
            }
            buf.push_back(sample);
        }
        Some(sample)
    }
}

impl<S> Source for TappedSource<S>
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
        let result = self.inner.try_seek(pos);
        if result.is_ok() {
            if let Ok(mut buf) = self.buf.lock() {
                buf.clear();
            }
        }
        result
    }
}

// Rounded gauge widget
struct RoundedGauge<'a> {
    ratio: f64,
    label: String,
    filled_color: Color,
    block: Option<Block<'a>>,
}

impl<'a> RoundedGauge<'a> {
    fn new(ratio: f64, label: String, filled_color: Color) -> Self {
        RoundedGauge {
            ratio: ratio.clamp(0.0, 1.0),
            label,
            filled_color,
            block: None,
        }
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
        let y = inner.y;

        for col in 0..width {
            let x = inner.x + col as u16;
            let (ch, fg, bg) = if filled == 0 {
                // Empty bar
                if col == 0 {
                    ('╶', Color::DarkGray, Color::Reset)
                } else if col == width - 1 {
                    ('╴', Color::DarkGray, Color::Reset)
                } else {
                    ('─', Color::DarkGray, Color::Reset)
                }
            } else if col < filled {
                // Filled region
                if col == 0 {
                    ('╺', self.filled_color, Color::Reset)
                } else if col == filled - 1 && filled < width {
                    ('╸', self.filled_color, Color::Reset)
                } else {
                    ('━', self.filled_color, Color::Reset)
                }
            } else {
                // Unfilled region
                if col == width - 1 {
                    ('╴', Color::DarkGray, Color::Reset)
                } else {
                    ('─', Color::DarkGray, Color::Reset)
                }
            };

            buf[(x, y)].set_char(ch).set_fg(fg).set_bg(bg);
        }

        // Center the label
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

// Bar visualization widget
const BAR_CHARS: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

struct VisualizerWidget<'a> {
    samples: &'a SampleBuf,
    block: Option<Block<'a>>,
}

impl<'a> VisualizerWidget<'a> {
    fn new(samples: &'a SampleBuf) -> Self {
        VisualizerWidget {
            samples,
            block: None,
        }
    }

    fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl Widget for VisualizerWidget<'_> {
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

        let num_bars = inner.width as usize;
        let max_height = inner.height as usize;

        for (i, col) in (0..num_bars).enumerate() {
            // Compute RMS amplitude for this chunk
            let start = (i * samples.len()) / num_bars;
            let end = ((i + 1) * samples.len()) / num_bars;
            let chunk = &samples[start.min(samples.len())..end.min(samples.len())];

            let rms = if chunk.is_empty() {
                0.0
            } else {
                let sum: f32 = chunk.iter().map(|s| s * s).sum();
                (sum / chunk.len() as f32).sqrt()
            };

            // Scale RMS to bar height (RMS of music is typically 0.0..0.3)
            let normalized = (rms * 4.0).min(1.0);
            let total_eighths = (normalized * (max_height * 8) as f32) as usize;
            let full_rows = total_eighths / 8;
            let remainder = total_eighths % 8;

            let x = inner.x + col as u16;

            // Draw from bottom up
            for row in 0..max_height {
                let y = inner.y + (max_height - 1 - row) as u16;
                let ch = if row < full_rows {
                    '█'
                } else if row == full_rows && remainder > 0 {
                    BAR_CHARS[remainder]
                } else {
                    ' '
                };

                // Color gradient: green at bottom, yellow in middle, red at top
                let color = if row < max_height / 6 {
                    Color::Green
                } else if row < max_height / 6 + max_height / 4 {
                    Color::Yellow
                } else {
                    Color::Red
                };

                buf[(x, y)].set_char(ch).set_fg(color);
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
    samples: SampleBuf,
    stream: OutputStream,
}

impl App {
    fn position(&self) -> Duration {
        self.seek_base + self.sink.get_pos()
    }
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
        let sink = Sink::connect_new(stream.mixer());

        let samples: SampleBuf = Arc::new(Mutex::new(VecDeque::with_capacity(SAMPLE_BUF_SIZE)));

        let file = fs::File::open(path).expect("failed to open file");
        let buf = io::BufReader::new(file);
        let source = Decoder::new(buf).expect("failed to decode audio file");
        let tapped = TappedSource::new(source, Arc::clone(&samples));
        sink.append(tapped);

        App {
            file_path: path.clone(),
            file_name,
            sink,
            paused: false,
            volume: 1.0,
            total_duration,
            seek_base: Duration::ZERO,
            samples,
            stream,
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
    }

    fn volume_down(&mut self) {
        self.volume = ((self.volume * 20.0).round() - 1.0).max(0.0) / 20.0;
        self.sink.set_volume(self.volume);
    }

    fn seek(&mut self, offset: i64) {
        let current = self.position();
        let target = if offset >= 0 {
            current + Duration::from_secs(offset as u64)
        } else {
            current.saturating_sub(Duration::from_secs((-offset) as u64))
        };
        let clamped = self.total_duration.map(|t| target.min(t)).unwrap_or(target);

        // Drop old sink and create a fresh one to avoid clear() issues
        self.sink.stop();
        let new_sink = Sink::connect_new(self.stream.mixer());
        new_sink.set_volume(self.volume);

        let file = fs::File::open(&self.file_path).expect("failed to open file");
        let buf = io::BufReader::new(file);
        let mut source = Decoder::new(buf).expect("failed to decode audio file");
        let _ = source.try_seek(clamped);
        let tapped = TappedSource::new(source, Arc::clone(&self.samples));
        new_sink.append(tapped);

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
        std::process::exit(1);
    }
    let path = PathBuf::from(&args[1]);
    if !path.exists() {
        eprintln!("File not found: {}", path.display());
        std::process::exit(1);
    }

    let mut terminal = ratatui::init();
    let mut app = App::new(&path);
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
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
    let vis = VisualizerWidget::new(&app.samples)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Visualizer "));
    frame.render_widget(vis, chunks[1]);

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
    let vol_ratio = (app.volume.min(1.0)) as f64;
    let vol_gauge = RoundedGauge::new(vol_ratio, format!("{}%", vol_pct), Color::Green)
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
        Span::styled(" q ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Quit"),
    ]))
    .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Controls "));
    frame.render_widget(help, chunks[4]);
}
