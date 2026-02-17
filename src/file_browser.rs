use std::path::{Path, PathBuf};

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState},
    Frame,
};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::theme::Theme;

pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "wav", "aac", "m4a"];

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

pub fn scan_directory(root: &Path) -> Vec<TreeItem<'static, PathBuf>> {
    let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(root) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return Vec::new(),
    };
    entries.sort_by(|a, b| {
        let a_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        b_dir.cmp(&a_dir).then_with(|| {
            a.file_name()
                .to_ascii_lowercase()
                .cmp(&b.file_name().to_ascii_lowercase())
        })
    });

    let mut items = Vec::new();
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            let children = scan_directory(&path);
            if !children.is_empty() {
                if let Ok(item) = TreeItem::new(path, name, children) {
                    items.push(item);
                }
            }
        } else if is_audio_file(&path) {
            items.push(TreeItem::new_leaf(path, name));
        }
    }
    items
}

/// Collect all audio file paths from the tree in display order (depth-first).
pub fn collect_audio_files(items: &[TreeItem<'static, PathBuf>]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    fn walk(items: &[TreeItem<'_, PathBuf>], out: &mut Vec<PathBuf>) {
        for item in items {
            let path = item.identifier();
            if path.is_file() && is_audio_file(path) {
                out.push(path.clone());
            }
            walk(item.children(), out);
        }
    }
    walk(items, &mut files);
    files
}

/// Fuzzy match: query chars must appear in order (case-insensitive).
fn fuzzy_match(query: &str, haystack: &str) -> bool {
    let mut chars = query.chars().flat_map(|c| c.to_lowercase());
    let mut current = match chars.next() {
        Some(c) => c,
        None => return true,
    };
    for h in haystack.chars().flat_map(|c| c.to_lowercase()) {
        if h == current {
            current = match chars.next() {
                Some(c) => c,
                None => return true,
            };
        }
    }
    false
}

/// Filter audio files by fuzzy matching against filenames. Returns matching paths.
pub fn filter_files(items: &[TreeItem<'static, PathBuf>], query: &str) -> Vec<PathBuf> {
    let all = collect_audio_files(items);
    if query.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
            fuzzy_match(query, &name)
        })
        .collect()
}

pub fn selected_file(state: &TreeState<PathBuf>) -> Option<PathBuf> {
    let selected = state.selected();
    let path = selected.last()?;
    if path.is_file() && is_audio_file(path) {
        Some(path.clone())
    } else {
        None
    }
}

fn popup_area(frame: &Frame) -> Rect {
    let area = frame.area();
    let popup_width = (area.width * 80 / 100).max(40).min(area.width);
    let popup_height = (area.height * 80 / 100).max(10).min(area.height);
    let popup_x = area.width.saturating_sub(popup_width) / 2;
    let popup_y = area.height.saturating_sub(popup_height) / 2;
    Rect::new(popup_x, popup_y, popup_width, popup_height)
}

pub fn draw_file_browser(
    frame: &mut Frame,
    items: &[TreeItem<'static, PathBuf>],
    state: &mut TreeState<PathBuf>,
    searching: bool,
    search: &str,
    filtered: &[PathBuf],
    filter_idx: usize,
    root_dir: Option<&Path>,
    theme: &Theme,
) {
    let popup = popup_area(frame);
    frame.render_widget(Clear, popup);

    if !searching {
        // Normal tree view
        let tree = Tree::new(items)
            .expect("unique identifiers")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(" Files ")
                    .title_bottom(" Enter: Play  ←/→: Expand  /: Search  Esc: Close "),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        frame.render_stateful_widget(tree, popup, state);
    } else {
        // Search mode: list with search query in bottom border
        let list_items: Vec<ListItem> = filtered
            .iter()
            .map(|p| {
                let display = root_dir
                    .and_then(|r| p.strip_prefix(r).ok())
                    .map(|rel| rel.to_string_lossy().to_string())
                    .unwrap_or_else(|| {
                        p.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default()
                    });
                ListItem::new(display)
            })
            .collect();

        let match_count = filtered.len();
        let bottom_title = Line::from(vec![
            Span::styled(" / ", Style::default().fg(Color::Black).bg(theme.secondary)),
            Span::styled(
                format!(" {search}█"),
                Style::default().fg(theme.text),
            ),
            Span::styled(
                format!("  ({match_count} matches) "),
                Style::default().fg(theme.dimmed),
            ),
        ]);

        let list = List::new(list_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(" Files ")
                    .title_bottom(bottom_title),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        let mut list_state = ListState::default();
        if !filtered.is_empty() {
            list_state.select(Some(filter_idx));
        }
        frame.render_stateful_widget(list, popup, &mut list_state);
    }
}
