use std::{sync::mpsc, thread};

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget},
    Frame,
};

use crate::TrackMeta;

pub const ART_ROWS: u16 = 16;
pub const ART_COLS: u16 = ART_ROWS * 2; // 2 cols per row for square aspect

// Album art pixel grid: rows of (R, G, B) tuples
pub type ArtPixels = Vec<Vec<(u8, u8, u8)>>;

pub fn fetch_album_art(url: &str, cols: u16, rows: u16) -> Option<ArtPixels> {
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

pub fn spawn_art_fetch(url: String, cols: u16, rows: u16) -> mpsc::Receiver<ArtPixels> {
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

/// Result of drawing the Now Playing panel.
pub struct NowPlayingLayout {
    /// Hit region for mouse clicks on the Now Playing area.
    pub region: Rect,
    /// Remaining area for the rest of the UI (right side or full area).
    pub main_area: Rect,
    /// Height of the now-playing row in the vertical layout (0 when art panel is shown).
    pub row_height: u16,
}

/// Draw the Now Playing panel and return layout info for the rest of the UI.
pub fn draw_now_playing(
    frame: &mut Frame,
    paused: bool,
    file_name: &str,
    meta: &TrackMeta,
    album_art: Option<&ArtPixels>,
    track_pos: Option<(usize, usize)>,
) -> NowPlayingLayout {
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
    let has_art = album_art.is_some();

    if has_art {
        let top_split = Layout::horizontal([
            Constraint::Length(ART_COLS + 2), // art + border
            Constraint::Min(0),
        ])
        .split(frame.area());

        let np_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Now Playing ");
        let np_inner = np_block.inner(top_split[0]);
        frame.render_widget(np_block, top_split[0]);

        // Status + title at the top
        let status = if paused { "Paused" } else { "Playing" };
        let mut status_spans = vec![Span::styled(
            format!(" {status} "),
            Style::default().fg(Color::Black).bg(Color::Cyan),
        )];
        if let Some((cur, total)) = track_pos {
            status_spans.push(Span::styled(
                format!("  {cur}/{total}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        let status_line = Line::from(status_spans);
        let title_line =
            Line::from(Span::styled(file_name, Style::default().fg(Color::White)));

        let status_rect = Rect::new(np_inner.x, np_inner.y, np_inner.width, 1);
        frame.render_widget(Paragraph::new(status_line), status_rect);
        let title_rect = Rect::new(np_inner.x, np_inner.y + 1, np_inner.width, 1);
        frame.render_widget(Paragraph::new(title_line), title_rect);

        // Album art below (after 1 blank line)
        let art_y = np_inner.y + 3;
        if let Some(pixels) = album_art {
            let art_rect = Rect::new(
                np_inner.x,
                art_y,
                ART_COLS.min(np_inner.width),
                ART_ROWS.min(np_inner.height.saturating_sub(3)),
            );
            frame.render_widget(AlbumArtWidget::new(pixels), art_rect);
        }

        // Tags below art
        let tags_y = art_y + ART_ROWS + 1;
        if has_meta && tags_y < np_inner.y + np_inner.height {
            let tags_rect = Rect::new(
                np_inner.x,
                tags_y,
                np_inner.width,
                np_inner.y + np_inner.height - tags_y,
            );
            let mut tag_lines: Vec<Line> = Vec::new();
            if let Some(ref artist) = meta.artist {
                tag_lines.push(Line::from(Span::styled(
                    artist.as_str(),
                    Style::default().fg(Color::White),
                )));
            }
            if let Some(ref album) = meta.album {
                tag_lines.push(Line::from(Span::styled(
                    album.as_str(),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            if let Some(ref date) = meta.date {
                tag_lines.push(Line::from(Span::styled(
                    date.as_str(),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            if let Some(ref genre) = meta.genre {
                tag_lines.push(Line::from(Span::styled(
                    genre.as_str(),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            frame.render_widget(Paragraph::new(tag_lines), tags_rect);
        }

        NowPlayingLayout {
            region: top_split[0],
            main_area: top_split[1],
            row_height: 0,
        }
    } else {
        let row_height: u16 = if has_meta { 4 } else { 3 };

        NowPlayingLayout {
            region: Rect::default(), // set later by caller after layout split
            main_area: frame.area(),
            row_height,
        }
    }
}

/// Draw the compact horizontal Now Playing bar (used when there's no album art).
pub fn draw_now_playing_bar(
    frame: &mut Frame,
    area: Rect,
    paused: bool,
    file_name: &str,
    meta: &TrackMeta,
    track_pos: Option<(usize, usize)>,
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
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw("  "),
        Span::styled(file_name, Style::default().fg(Color::White)),
    ];
    if let Some((cur, total)) = track_pos {
        title_spans.push(Span::styled(
            format!("  {cur}/{total}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let mut lines = vec![Line::from(title_spans)];
    if has_meta {
        lines.push(Line::from(vec![
            Span::raw("         "),
            Span::styled(
                meta_parts.join("  ·  "),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }
    let title = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Now Playing "),
    );
    frame.render_widget(title, area);
}
