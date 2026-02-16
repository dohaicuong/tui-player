use std::{sync::mpsc, thread};

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

pub struct LyricsResult {
    pub text: String,
    pub url: String,
    pub art_url: Option<String>,
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

pub fn spawn_lyrics_fetchers(artist: String, title: String) -> mpsc::Receiver<Option<LyricsResult>> {
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

/// Draw the expanded lyrics panel.
pub fn draw_lyrics(
    frame: &mut Frame,
    area: Rect,
    lyrics: Option<&LyricsResult>,
    lyrics_url: &str,
    lyrics_loading: bool,
    lyrics_scroll: &mut usize,
) {
    let lyrics_text = if lyrics_loading {
        format!("Loading...\n\n{}", lyrics_url)
    } else if let Some(lr) = lyrics {
        lr.text.clone()
    } else {
        "No lyrics found".to_string()
    };

    let mut lyrics_lines: Vec<Line> = Vec::new();
    if !lyrics_url.is_empty() {
        lyrics_lines.push(Line::from(Span::styled(lyrics_url, Style::default().fg(Color::DarkGray))));
        lyrics_lines.push(Line::raw(""));
    }
    lyrics_lines.extend(lyrics_text.lines().map(|l| Line::raw(l)));
    let total_lines = lyrics_lines.len();
    let visible_height = area.height.saturating_sub(2) as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    *lyrics_scroll = (*lyrics_scroll).min(max_scroll);

    let lyrics_widget = Paragraph::new(lyrics_lines)
        .scroll((*lyrics_scroll as u16, 0))
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Lyrics "),
        );
    frame.render_widget(lyrics_widget, area);
}

/// Draw the collapsed vertical lyrics tab.
pub fn draw_lyrics_collapsed(frame: &mut Frame, area: Rect) {
    let inner_h = area.height.saturating_sub(2) as usize;
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
    frame.render_widget(collapsed, area);
}
