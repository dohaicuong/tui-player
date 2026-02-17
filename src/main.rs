use std::{
    collections::VecDeque,
    env, fs, io,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
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
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::Span,
    widgets::Paragraph,
    DefaultTerminal, Frame,
};
use tui_tree_widget::{TreeItem, TreeState};
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
mod now_playing;
use now_playing::{spawn_art_fetch, ArtPixels, ART_COLS, ART_ROWS};

mod visualizer;
use visualizer::VisMode;

mod lyrics;
use lyrics::{spawn_lyrics_fetchers, LyricsResult};

mod eq;
mod file_browser;
mod gauge;
mod progress;
mod volume;
mod controls;

const PIPE_PATH: &str = "/tmp/tui-player.pipe";

pub type SampleBuf = Arc<Mutex<VecDeque<f32>>>;
const SAMPLE_BUF_SIZE: usize = 8192;

// Source wrapper that applies EQ, writes to pipe, and captures samples for visualization
struct PipedSource<S> {
    inner: S,
    pipe: Option<fs::File>,
    pipe_ready: Arc<AtomicBool>,
    samples: SampleBuf,
    eq_params: eq::SharedEqParams,
    eq_filters: eq::EqFilters,
    channel_idx: u16,
    channels: u16,
    update_counter: u32,
    finished: Arc<AtomicBool>,
}

impl<S> PipedSource<S>
where
    S: Source<Item = f32>,
{
    fn new(
        source: S,
        pipe_ready: Arc<AtomicBool>,
        samples: SampleBuf,
        eq_params: eq::SharedEqParams,
        channels: u16,
        sample_rate: u32,
        finished: Arc<AtomicBool>,
    ) -> Self {
        let eq_filters = {
            let params = eq_params.lock().unwrap();
            eq::EqFilters::new(channels, sample_rate as f32, &params)
        };
        PipedSource {
            inner: source,
            pipe: None,
            pipe_ready,
            samples,
            eq_params,
            eq_filters,
            channel_idx: 0,
            channels,
            update_counter: 0,
            finished,
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
        let raw = match self.inner.next() {
            Some(v) => v,
            None => {
                self.finished.store(true, Ordering::Relaxed);
                return None;
            }
        };

        // Periodically check for EQ parameter changes (every 4096 samples)
        self.update_counter += 1;
        if self.update_counter >= 4096 {
            self.update_counter = 0;
            if let Ok(params) = self.eq_params.try_lock() {
                self.eq_filters.update_if_changed(&params);
            }
        }

        // Apply EQ
        let sample = self.eq_filters.process(raw, self.channel_idx as usize);
        self.channel_idx = (self.channel_idx + 1) % self.channels;

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

#[derive(Default, Clone)]
struct LayoutRegions {
    now_playing: Rect,
    progress: Rect,
    volume: Rect,
    visualizer: Rect,
    lyrics: Rect,
    lyrics_title: Rect,
}

struct QueuedTrack {
    path: PathBuf,
    file_name: String,
    meta: TrackMeta,
    duration: Option<Duration>,
    channels: u16,
    finished: Arc<AtomicBool>,
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
    album_art: Option<ArtPixels>,
    art_rx: Option<mpsc::Receiver<ArtPixels>>,
    root_dir: Option<PathBuf>,
    browser_open: bool,
    browser_state: TreeState<PathBuf>,
    browser_items: Vec<TreeItem<'static, PathBuf>>,
    browser_searching: bool,
    browser_search: String,
    browser_filtered: Vec<PathBuf>,
    browser_filter_idx: usize,
    track_loaded: bool,
    current_finished: Arc<AtomicBool>,
    queued_track: Option<QueuedTrack>,
    eq_open: bool,
    eq_params: eq::SharedEqParams,
    eq_selected_band: usize,
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
pub struct TrackMeta {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub date: Option<String>,
    pub genre: Option<String>,
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
    fn new_with_track(path: &PathBuf, root_dir: Option<PathBuf>) -> Self {
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
        let eq_params = Arc::new(Mutex::new(eq::load_eq()));

        let file = fs::File::open(path).expect("failed to open file");
        let buf = io::BufReader::new(file);
        let source = Decoder::new(buf).expect("failed to decode audio file");
        let channels = source.channels();
        let sample_rate = source.sample_rate();
        let current_finished = Arc::new(AtomicBool::new(false));
        let piped = PipedSource::new(
            source,
            Arc::clone(&pipe_ready),
            Arc::clone(&samples),
            Arc::clone(&eq_params),
            channels,
            sample_rate,
            Arc::clone(&current_finished),
        );
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

        let browser_items = root_dir
            .as_ref()
            .map(|d| file_browser::scan_directory(d))
            .unwrap_or_default();

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
            root_dir,
            browser_open: false,
            browser_state: TreeState::default(),
            browser_items,
            browser_searching: false,
            browser_search: String::new(),
            browser_filtered: Vec::new(),
            browser_filter_idx: 0,
            track_loaded: true,
            current_finished,
            queued_track: None,
            eq_open: false,
            eq_params,
            eq_selected_band: 0,
        }
    }

    fn new_idle(root_dir: PathBuf) -> Self {
        let stream = OutputStreamBuilder::from_default_device()
            .expect("failed to find audio device")
            .open_stream_or_fallback()
            .expect("failed to open audio stream");
        let volume = load_volume();
        let sink = Sink::connect_new(stream.mixer());
        sink.set_volume(volume);

        let pipe_ready = Arc::new(AtomicBool::new(true));
        let samples: SampleBuf = Arc::new(Mutex::new(VecDeque::with_capacity(SAMPLE_BUF_SIZE)));
        let eq_params = Arc::new(Mutex::new(eq::load_eq()));

        let browser_items = file_browser::scan_directory(&root_dir);
        let mut browser_state = TreeState::default();
        browser_state.select_first();

        App {
            file_path: PathBuf::new(),
            file_name: String::new(),
            sink,
            paused: true,
            volume,
            total_duration: None,
            seek_base: Duration::ZERO,
            channels: 2,
            pipe_ready,
            samples,
            stream,
            vis_mode: load_vis_mode(),
            show_visualizer: true,
            meta: TrackMeta::default(),
            regions: LayoutRegions::default(),
            lyrics: None,
            lyrics_scroll: 0,
            lyrics_visible: load_lyrics_visible(),
            lyrics_loading: false,
            lyrics_url: String::new(),
            lyrics_rx: None,
            album_art: None,
            art_rx: None,
            root_dir: Some(root_dir),
            browser_open: true,
            browser_state,
            browser_items,
            browser_searching: false,
            browser_search: String::new(),
            browser_filtered: Vec::new(),
            browser_filter_idx: 0,
            track_loaded: false,
            current_finished: Arc::new(AtomicBool::new(false)),
            queued_track: None,
            eq_open: false,
            eq_params,
            eq_selected_band: 0,
        }
    }

    fn switch_track(&mut self, path: &PathBuf) {
        self.sink.stop();
        self.queued_track = None;

        let probe = probe_file(path);
        self.file_name = probe.meta.title.clone().unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".into())
        });
        self.total_duration = probe.duration;
        self.file_path = path.clone();
        self.seek_base = Duration::ZERO;
        self.paused = false;

        let new_sink = Sink::connect_new(self.stream.mixer());
        new_sink.set_volume(self.volume);

        let file = fs::File::open(path).expect("failed to open file");
        let buf = io::BufReader::new(file);
        let source = Decoder::new(buf).expect("failed to decode audio file");
        self.channels = source.channels();
        let sample_rate = source.sample_rate();
        self.current_finished = Arc::new(AtomicBool::new(false));
        let piped = PipedSource::new(
            source,
            Arc::clone(&self.pipe_ready),
            Arc::clone(&self.samples),
            Arc::clone(&self.eq_params),
            self.channels,
            sample_rate,
            Arc::clone(&self.current_finished),
        );
        new_sink.append(piped);
        self.sink = new_sink;

        if let Ok(mut sbuf) = self.samples.lock() {
            sbuf.clear();
        }

        // Reset lyrics and art
        self.lyrics = None;
        self.lyrics_scroll = 0;
        self.lyrics_loading = false;
        self.lyrics_url.clear();
        self.lyrics_rx = None;
        self.album_art = None;
        self.art_rx = None;

        // Spawn new lyrics fetchers
        let lyrics_artist = probe.meta.artist.clone().unwrap_or_default();
        let lyrics_title = probe.meta.title.clone().unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });
        if !lyrics_title.is_empty() {
            self.lyrics_rx = Some(spawn_lyrics_fetchers(lyrics_artist, lyrics_title));
            self.lyrics_loading = true;
        }

        self.meta = probe.meta;
        self.track_loaded = true;
        self.queue_next_track();
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
        self.queued_track = None;

        let new_sink = Sink::connect_new(self.stream.mixer());
        new_sink.set_volume(self.volume);

        let file = fs::File::open(&self.file_path).expect("failed to open file");
        let buf = io::BufReader::new(file);
        let mut source = Decoder::new(buf).expect("failed to decode audio file");
        let sample_rate = source.sample_rate();
        let _ = source.try_seek(clamped);
        self.current_finished = Arc::new(AtomicBool::new(false));
        let piped = PipedSource::new(
            source,
            Arc::clone(&self.pipe_ready),
            Arc::clone(&self.samples),
            Arc::clone(&self.eq_params),
            self.channels,
            sample_rate,
            Arc::clone(&self.current_finished),
        );
        new_sink.append(piped);

        if self.paused {
            new_sink.pause();
        }

        self.sink = new_sink;
        self.seek_base = clamped;

        if let Ok(mut sbuf) = self.samples.lock() {
            sbuf.clear();
        }

        self.queue_next_track();
    }

    fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 2.0);
        // Snap to 5% grid
        self.volume = (self.volume * 20.0).round() / 20.0;
        self.sink.set_volume(self.volume);
        save_volume(self.volume);
    }

    fn next_track(&mut self) {
        let files = file_browser::collect_audio_files(&self.browser_items);
        let idx = files.iter().position(|f| f == &self.file_path);
        if let Some(next) = idx.and_then(|i| files.get(i + 1)).cloned() {
            self.switch_track(&next);
        }
    }

    fn prev_track(&mut self) {
        let files = file_browser::collect_audio_files(&self.browser_items);
        let idx = files.iter().position(|f| f == &self.file_path);
        if let Some(prev) = idx.and_then(|i| i.checked_sub(1)).and_then(|i| files.get(i)).cloned()
        {
            self.switch_track(&prev);
        }
    }

    fn queue_next_track(&mut self) {
        if self.queued_track.is_some() {
            return;
        }
        let files = file_browser::collect_audio_files(&self.browser_items);
        let next_path = match files.iter().position(|f| f == &self.file_path) {
            Some(i) => match files.get(i + 1) {
                Some(p) => p.clone(),
                None => return,
            },
            None => return,
        };

        let probe = probe_file(&next_path);
        let file_name = probe.meta.title.clone().unwrap_or_else(|| {
            next_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".into())
        });

        let file = match fs::File::open(&next_path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let buf = io::BufReader::new(file);
        let source = match Decoder::new(buf) {
            Ok(s) => s,
            Err(_) => return,
        };
        let channels = source.channels();
        let sample_rate = source.sample_rate();
        let finished = Arc::new(AtomicBool::new(false));
        let piped = PipedSource::new(
            source,
            Arc::clone(&self.pipe_ready),
            Arc::clone(&self.samples),
            Arc::clone(&self.eq_params),
            channels,
            sample_rate,
            Arc::clone(&finished),
        );
        self.sink.append(piped);

        self.queued_track = Some(QueuedTrack {
            path: next_path,
            file_name,
            meta: probe.meta,
            duration: probe.duration,
            channels,
            finished,
        });
    }

    fn advance_to_queued(&mut self) {
        let queued = match self.queued_track.take() {
            Some(q) => q,
            None => return,
        };

        self.file_path = queued.path;
        self.file_name = queued.file_name;
        self.total_duration = queued.duration;
        self.seek_base = Duration::ZERO;
        self.channels = queued.channels;
        self.current_finished = queued.finished;

        // Reset lyrics and art
        self.lyrics = None;
        self.lyrics_scroll = 0;
        self.lyrics_loading = false;
        self.lyrics_url.clear();
        self.lyrics_rx = None;
        self.album_art = None;
        self.art_rx = None;

        // Spawn new lyrics fetchers
        let lyrics_artist = queued.meta.artist.clone().unwrap_or_default();
        let lyrics_title = queued.meta.title.clone().unwrap_or_else(|| {
            self.file_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });
        if !lyrics_title.is_empty() {
            self.lyrics_rx = Some(spawn_lyrics_fetchers(lyrics_artist, lyrics_title));
            self.lyrics_loading = true;
        }

        self.meta = queued.meta;

        // Queue the next-next track
        self.queue_next_track();
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
        eprintln!("Usage: tui-player <music-file-or-directory>");
        if scope_tui_installed {
            eprintln!();
            eprintln!("For external visualization, run in another terminal:");
            eprintln!("  scope-tui file {PIPE_PATH}");
        }
        std::process::exit(1);
    }
    let path = PathBuf::from(&args[1]);
    if !path.exists() {
        eprintln!("Path not found: {}", path.display());
        std::process::exit(1);
    }

    if scope_tui_installed {
        create_pipe();
    }

    crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;
    let mut terminal = ratatui::init();
    let mut app = if path.is_dir() {
        App::new_idle(path)
    } else {
        let root_dir = path.parent().map(|p| p.to_path_buf());
        App::new_with_track(&path, root_dir)
    };
    app.show_visualizer = scope_tui_installed;
    app.queue_next_track();
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
        if app.track_loaded {
            // Poll lyrics results — first Some wins, keep trying until all sources done
            if let Some(ref rx) = app.lyrics_rx {
                loop {
                    match rx.try_recv() {
                        Ok(Some(lr)) => {
                            app.lyrics_url = lr.url.clone();
                            if let Some(ref art_url) = lr.art_url {
                                app.art_rx =
                                    Some(spawn_art_fetch(art_url.clone(), ART_COLS, ART_ROWS));
                            }
                            app.lyrics = Some(lr);
                            app.lyrics_loading = false;
                            app.lyrics_rx = None;
                            break;
                        }
                        Ok(None) => {}
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
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
        }

        terminal.draw(|f| draw(f, &mut *app))?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Quit always works
                    if key.code == KeyCode::Char('q')
                        || (key.code == KeyCode::Char('c')
                            && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL))
                    {
                        break;
                    }

                    if app.browser_open {
                        if app.browser_searching {
                            // Search mode keys
                            match key.code {
                                KeyCode::Esc => {
                                    app.browser_searching = false;
                                    app.browser_search.clear();
                                    app.browser_filtered.clear();
                                    app.browser_filter_idx = 0;
                                }
                                KeyCode::Backspace => {
                                    app.browser_search.pop();
                                    app.browser_filtered = file_browser::filter_files(
                                        &app.browser_items,
                                        &app.browser_search,
                                    );
                                    if app.browser_filter_idx >= app.browser_filtered.len() {
                                        app.browser_filter_idx =
                                            app.browser_filtered.len().saturating_sub(1);
                                    }
                                }
                                KeyCode::Up => {
                                    app.browser_filter_idx =
                                        app.browser_filter_idx.saturating_sub(1);
                                }
                                KeyCode::Down => {
                                    if !app.browser_filtered.is_empty() {
                                        app.browser_filter_idx = (app.browser_filter_idx + 1)
                                            .min(app.browser_filtered.len() - 1);
                                    }
                                }
                                KeyCode::Enter => {
                                    if let Some(path) =
                                        app.browser_filtered.get(app.browser_filter_idx).cloned()
                                    {
                                        app.switch_track(&path);
                                        app.browser_open = false;
                                        app.browser_searching = false;
                                        app.browser_search.clear();
                                        app.browser_filtered.clear();
                                        app.browser_filter_idx = 0;
                                    }
                                }
                                KeyCode::Char(c) => {
                                    app.browser_search.push(c);
                                    app.browser_filtered = file_browser::filter_files(
                                        &app.browser_items,
                                        &app.browser_search,
                                    );
                                    app.browser_filter_idx = 0;
                                }
                                _ => {}
                            }
                        } else {
                            // Normal tree mode keys
                            match key.code {
                                KeyCode::Up => {
                                    app.browser_state.key_up();
                                }
                                KeyCode::Down => {
                                    app.browser_state.key_down();
                                }
                                KeyCode::Left => {
                                    app.browser_state.key_left();
                                }
                                KeyCode::Right => {
                                    app.browser_state.key_right();
                                }
                                KeyCode::Enter => {
                                    if let Some(path) =
                                        file_browser::selected_file(&app.browser_state)
                                    {
                                        app.switch_track(&path);
                                        app.browser_open = false;
                                    } else {
                                        app.browser_state.toggle_selected();
                                    }
                                }
                                KeyCode::Char('/') => {
                                    app.browser_searching = true;
                                    app.browser_search.clear();
                                    app.browser_filtered = file_browser::filter_files(
                                        &app.browser_items,
                                        "",
                                    );
                                    app.browser_filter_idx = 0;
                                }
                                KeyCode::Esc | KeyCode::Char('f') => {
                                    if app.track_loaded {
                                        app.browser_open = false;
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else if app.eq_open {
                        match key.code {
                            KeyCode::Left => {
                                app.eq_selected_band =
                                    app.eq_selected_band.saturating_sub(1);
                            }
                            KeyCode::Right => {
                                app.eq_selected_band =
                                    (app.eq_selected_band + 1).min(eq::NUM_BANDS - 1);
                            }
                            KeyCode::Up => {
                                if let Ok(mut params) = app.eq_params.lock() {
                                    let g = &mut params.gains[app.eq_selected_band];
                                    *g = (*g + 1.0).min(12.0);
                                    eq::save_eq(&params);
                                }
                            }
                            KeyCode::Down => {
                                if let Ok(mut params) = app.eq_params.lock() {
                                    let g = &mut params.gains[app.eq_selected_band];
                                    *g = (*g - 1.0).max(-12.0);
                                    eq::save_eq(&params);
                                }
                            }
                            KeyCode::Char('p') => {
                                if let Ok(mut params) = app.eq_params.lock() {
                                    params.preset_index =
                                        (params.preset_index + 1) % eq::PRESETS.len();
                                    params.gains = eq::PRESETS[params.preset_index].1;
                                    eq::save_eq(&params);
                                }
                            }
                            KeyCode::Char('0') => {
                                if let Ok(mut params) = app.eq_params.lock() {
                                    params.gains = [0.0; eq::NUM_BANDS];
                                    params.preset_index = 0;
                                    eq::save_eq(&params);
                                }
                            }
                            KeyCode::Char('s') => {
                                if let Ok(mut params) = app.eq_params.lock() {
                                    params.enabled = !params.enabled;
                                    eq::save_eq(&params);
                                }
                            }
                            KeyCode::Esc | KeyCode::Char('e') => {
                                app.eq_open = false;
                            }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char(' ') => {
                                if app.track_loaded {
                                    app.toggle_pause();
                                }
                            }
                            KeyCode::Up => app.volume_up(),
                            KeyCode::Down => app.volume_down(),
                            KeyCode::Right => {
                                if app.track_loaded {
                                    app.seek(5);
                                }
                            }
                            KeyCode::Left => {
                                if app.track_loaded {
                                    app.seek(-5);
                                }
                            }
                            KeyCode::Char('v') => {
                                app.vis_mode = app.vis_mode.next();
                                save_vis_mode(app.vis_mode);
                            }
                            KeyCode::Char('l') => {
                                app.lyrics_visible = !app.lyrics_visible;
                                save_lyrics_visible(app.lyrics_visible);
                            }
                            KeyCode::Char('j') => {
                                app.lyrics_scroll = app.lyrics_scroll.saturating_add(1);
                            }
                            KeyCode::Char('k') => {
                                app.lyrics_scroll = app.lyrics_scroll.saturating_sub(1);
                            }
                            KeyCode::Char('f') => {
                                if app.root_dir.is_some() {
                                    app.browser_open = true;
                                }
                            }
                            KeyCode::Char('e') => {
                                app.eq_open = true;
                            }
                            KeyCode::Char('n') => {
                                if app.track_loaded {
                                    app.next_track();
                                }
                            }
                            KeyCode::Char('N') => {
                                if app.track_loaded {
                                    app.prev_track();
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::Mouse(mouse) if !app.browser_open && !app.eq_open => {
                    let col = mouse.column;
                    let row = mouse.row;
                    match mouse.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            if hit(app.regions.now_playing, col, row) {
                                app.toggle_pause();
                            } else if hit(app.regions.progress, col, row) {
                                if let Some(total) = app.total_duration {
                                    let inner_x =
                                        col.saturating_sub(app.regions.progress.x + 1);
                                    let inner_w =
                                        app.regions.progress.width.saturating_sub(2);
                                    if inner_w > 0 {
                                        let frac = inner_x as f64 / inner_w as f64;
                                        let target = Duration::from_secs_f64(
                                            frac * total.as_secs_f64(),
                                        );
                                        app.seek_to(target);
                                    }
                                }
                            } else if hit(app.regions.volume, col, row) {
                                let inner_x =
                                    col.saturating_sub(app.regions.volume.x + 1);
                                let inner_w =
                                    app.regions.volume.width.saturating_sub(2);
                                if inner_w > 0 {
                                    let frac = inner_x as f64 / inner_w as f64;
                                    app.set_volume(frac as f32 * 2.0);
                                }
                            } else if (!app.lyrics_visible
                                && hit(app.regions.lyrics, col, row))
                                || (app.lyrics_visible
                                    && hit(app.regions.lyrics_title, col, row))
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

        // Gapless transition: current source finished, queued is now playing
        if app.track_loaded
            && app.current_finished.load(Ordering::Relaxed)
            && app.queued_track.is_some()
        {
            app.advance_to_queued();
        }

        // All sources exhausted (no queued track)
        if app.track_loaded && app.is_finished() && !app.paused {
            if app.root_dir.is_some() {
                app.browser_open = true;
                app.track_loaded = false;
            } else {
                break;
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame, app: &mut App) {
    if !app.track_loaded {
        // Idle screen — no track playing yet
        let area = frame.area();
        let msg = Paragraph::new(Span::styled(
            "No track playing — select a file from the browser",
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Center);
        let y = area.height / 2;
        frame.render_widget(msg, Rect::new(area.x, y, area.width, 1));
    } else {
        let track_pos = {
            let files = file_browser::collect_audio_files(&app.browser_items);
            files
                .iter()
                .position(|f| f == &app.file_path)
                .map(|i| (i + 1, files.len()))
        };
        let np = now_playing::draw_now_playing(
            frame,
            app.paused,
            &app.file_name,
            &app.meta,
            app.album_art.as_ref(),
            track_pos,
        );
        app.regions.now_playing = np.region;

        let show_middle = app.show_visualizer || app.lyrics_visible;
        let show_hint = !app.show_visualizer;

        let chunks = Layout::vertical([
            Constraint::Length(np.row_height),
            if show_middle {
                Constraint::Min(8)
            } else {
                Constraint::Length(0)
            },
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(if show_hint { 1 } else { 0 }),
        ])
        .split(np.main_area);

        app.regions.visualizer = chunks[1];
        app.regions.progress = chunks[2];
        app.regions.volume = chunks[3];

        if np.row_height > 0 {
            app.regions.now_playing = chunks[0];
            now_playing::draw_now_playing_bar(
                frame,
                chunks[0],
                app.paused,
                &app.file_name,
                &app.meta,
                track_pos,
            );
        }

        if show_middle {
            let collapsed_w: u16 = 3;
            let (vis_area, lyrics_rect) = if app.show_visualizer && app.lyrics_visible {
                let split = Layout::horizontal([
                    Constraint::Percentage(50),
                    Constraint::Percentage(50),
                ])
                .split(chunks[1]);
                (Some(split[0]), split[1])
            } else if app.show_visualizer {
                let split = Layout::horizontal([
                    Constraint::Min(0),
                    Constraint::Length(collapsed_w),
                ])
                .split(chunks[1]);
                (Some(split[0]), split[1])
            } else {
                (None, chunks[1])
            };

            if let Some(va) = vis_area {
                app.regions.visualizer = va;
            } else {
                app.regions.visualizer = Rect::default();
            }
            app.regions.lyrics = lyrics_rect;
            app.regions.lyrics_title =
                Rect::new(lyrics_rect.x, lyrics_rect.y, lyrics_rect.width, 1);

            if let Some(va) = vis_area {
                visualizer::draw_visualizer(frame, va, app.vis_mode, &app.samples, app.channels);
            }

            if app.lyrics_visible {
                lyrics::draw_lyrics(
                    frame,
                    lyrics_rect,
                    app.lyrics.as_ref(),
                    &app.lyrics_url,
                    app.lyrics_loading,
                    &mut app.lyrics_scroll,
                );
            } else if app.show_visualizer {
                lyrics::draw_lyrics_collapsed(frame, lyrics_rect);
            }
        }

        progress::draw_progress(frame, chunks[2], app.position(), app.total_duration);
        volume::draw_volume(frame, chunks[3], app.volume);
        controls::draw_controls(frame, chunks[4], app.show_visualizer, app.root_dir.is_some());
        if show_hint {
            controls::draw_scope_hint(frame, chunks[5]);
        }
    }

    // Overlays (rendered on top)
    if app.browser_open {
        file_browser::draw_file_browser(
            frame,
            &app.browser_items,
            &mut app.browser_state,
            app.browser_searching,
            &app.browser_search,
            &app.browser_filtered,
            app.browser_filter_idx,
            app.root_dir.as_deref(),
        );
    }
    if app.eq_open {
        let params = app.eq_params.lock().unwrap();
        eq::draw_eq(frame, &params, app.eq_selected_band);
    }
}
