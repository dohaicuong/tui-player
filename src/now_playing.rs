use std::{fs, path::PathBuf, sync::mpsc, thread};

use crate::{cache_hash, config_dir};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap},
    Frame,
};

use crate::TrackMeta;
use crate::theme::Theme;

pub const ART_ROWS: u16 = 16;
pub const ART_COLS: u16 = ART_ROWS * 2; // 2 cols per row for square aspect

// Album art pixel grid: rows of (R, G, B) tuples
pub type ArtPixels = Vec<Vec<(u8, u8, u8)>>;

fn art_cache_path(url: &str) -> PathBuf {
    config_dir()
        .join("cache")
        .join("art")
        .join(cache_hash(url))
}

pub fn fetch_album_art(url: &str, cols: u16, rows: u16) -> Option<ArtPixels> {
    let cache_path = art_cache_path(url);
    let bytes = if let Ok(cached) = fs::read(&cache_path) {
        cached
    } else {
        let downloaded = ureq::get(url).call().ok()?.body_mut().read_to_vec().ok()?;
        if let Some(parent) = cache_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&cache_path, &downloaded);
        downloaded
    };
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

pub fn spawn_art_fetch(url: String, cols: u16, rows: u16) -> mpsc::Receiver<ArtPixels> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Some(pixels) = fetch_album_art(&url, cols, rows) {
            let _ = tx.send(pixels);
        }
    });
    rx
}

fn color_to_rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (205, 0, 0),
        Color::Green => (0, 205, 0),
        Color::Yellow => (205, 205, 0),
        Color::Blue => (0, 0, 238),
        Color::Magenta => (205, 0, 205),
        Color::Cyan => (0, 205, 205),
        Color::White => (229, 229, 229),
        Color::DarkGray => (127, 127, 127),
        _ => (0, 0, 0),
    }
}

fn blend(art: u8, bg: u8, opacity: f32) -> u8 {
    (art as f32 * opacity + bg as f32 * (1.0 - opacity)) as u8
}

struct AlbumArtWidget<'a> {
    pixels: &'a [Vec<(u8, u8, u8)>],
    opacity: f32,
}

impl<'a> AlbumArtWidget<'a> {
    fn new(pixels: &'a [Vec<(u8, u8, u8)>], opacity: f32) -> Self {
        AlbumArtWidget { pixels, opacity }
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
                let existing = &buf[(x, y)];
                let ex_fg = color_to_rgb(existing.fg);
                let ex_bg = color_to_rgb(existing.bg);
                let o = self.opacity;
                buf[(x, y)]
                    .set_char('▀')
                    .set_fg(Color::Rgb(
                        blend(top.0, ex_fg.0, o),
                        blend(top.1, ex_fg.1, o),
                        blend(top.2, ex_fg.2, o),
                    ))
                    .set_bg(Color::Rgb(
                        blend(bot.0, ex_bg.0, o),
                        blend(bot.1, ex_bg.1, o),
                        blend(bot.2, ex_bg.2, o),
                    ));
            }
        }
    }
}

/// Return the height needed for the compact Now Playing bar, accounting for line wrapping.
pub fn now_playing_height(
    meta: &TrackMeta,
    file_name: &str,
    track_pos: Option<(usize, usize)>,
    width: u16,
) -> u16 {
    let inner_w = width.saturating_sub(2) as usize;
    if inner_w == 0 {
        return 3;
    }

    // Line 1: status + filename + track position
    let mut title_spans = vec![
        Span::raw(" Playing "),
        Span::raw("  "),
        Span::raw(file_name.to_string()),
    ];
    if let Some((cur, total)) = track_pos {
        title_spans.push(Span::raw(format!("  {cur}/{total}")));
    }
    let line1_w = Line::from(title_spans).width();
    let line1_rows = ((line1_w + inner_w - 1) / inner_w).max(1);

    // Line 2: metadata
    let mut meta_parts: Vec<&str> = Vec::new();
    if let Some(ref a) = meta.artist {
        meta_parts.push(a);
    }
    if let Some(ref a) = meta.album {
        meta_parts.push(a);
    }
    if let Some(ref d) = meta.date {
        meta_parts.push(d);
    }
    if let Some(ref g) = meta.genre {
        meta_parts.push(g);
    }
    let line2_rows = if meta_parts.is_empty() {
        0
    } else {
        let meta_str = meta_parts.join("  \u{00b7}  ");
        let line2_w = Line::from(vec![Span::raw("         "), Span::raw(meta_str)]).width();
        ((line2_w + inner_w - 1) / inner_w).max(1)
    };

    (line1_rows + line2_rows) as u16 + 2
}

/// Render album art as a small overlay on the top-left corner of a given area,
/// inset by 1 cell to sit inside the visualizer border.
pub fn draw_art_overlay(frame: &mut Frame, area: Rect, pixels: &ArtPixels, opacity: f32) {
    let inner_x = area.x + 1;
    let inner_y = area.y + 1;
    let inner_w = area.width.saturating_sub(2);
    let inner_h = area.height.saturating_sub(2);
    let art_w = ART_COLS.min(inner_w);
    let art_h = ART_ROWS.min(inner_h);
    if art_w == 0 || art_h == 0 {
        return;
    }
    let art_rect = Rect::new(inner_x, inner_y, art_w, art_h);
    frame.render_widget(AlbumArtWidget::new(pixels, opacity), art_rect);
}

/// Draw the compact horizontal Now Playing bar (used when there's no album art).
pub fn draw_now_playing_bar(
    frame: &mut Frame,
    area: Rect,
    paused: bool,
    file_name: &str,
    meta: &TrackMeta,
    track_pos: Option<(usize, usize)>,
    theme: &Theme,
) {
    let mut meta_parts: Vec<&str> = Vec::new();
    if let Some(ref artist) = meta.artist {
        meta_parts.push(artist);
    }
    if let Some(ref album) = meta.album {
        meta_parts.push(album);
    }
    if let Some(ref date) = meta.date {
        meta_parts.push(date);
    }
    if let Some(ref genre) = meta.genre {
        meta_parts.push(genre);
    }
    let has_meta = !meta_parts.is_empty();

    let status = if paused { "Paused" } else { "Playing" };
    let mut title_spans = vec![
        Span::styled(
            format!(" {status} "),
            Style::default().fg(Color::Black).bg(theme.accent),
        ),
        Span::raw("  "),
        Span::styled(file_name, Style::default().fg(theme.text)),
    ];
    if let Some((cur, total)) = track_pos {
        title_spans.push(Span::styled(
            format!("  {cur}/{total}"),
            Style::default().fg(theme.dimmed),
        ));
    }
    let mut lines = vec![Line::from(title_spans)];
    if has_meta {
        lines.push(Line::from(vec![
            Span::raw("         "),
            Span::styled(
                meta_parts.join("  ·  "),
                Style::default().fg(theme.dimmed),
            ),
        ]));
    }
    let title = Paragraph::new(lines)
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Now Playing "),
        );
    frame.render_widget(title, area);
}
