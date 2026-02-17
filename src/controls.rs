use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use crate::theme::Theme;

fn build_control_spans(
    show_visualizer: bool,
    has_browser: bool,
    shuffle: bool,
    repeat_label: &str,
    crossfade_label: &str,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let key_style = Style::default().fg(Color::Black).bg(theme.secondary);
    let mut spans = vec![
        Span::styled(" Space ", key_style),
        Span::raw(" Play/Pause  "),
        Span::styled(" ←/→ ", key_style),
        Span::raw(" Seek ±5s  "),
        Span::styled(" ↑/↓ ", key_style),
        Span::raw(" Volume  "),
    ];
    if show_visualizer {
        spans.extend([
            Span::styled(" v ", key_style),
            Span::raw(" Vis Mode  "),
        ]);
    }
    spans.extend([
        Span::styled(" l ", key_style),
        Span::raw(" Lyrics  "),
    ]);
    spans.extend([
        Span::styled(" e ", key_style),
        Span::raw(" EQ  "),
    ]);
    if has_browser {
        spans.extend([
            Span::styled(" n/N ", key_style),
            Span::raw(" Next/Prev  "),
            Span::styled(" s ", key_style),
            Span::styled(
                if shuffle { " Shuffle On  " } else { " Shuffle Off  " },
                Style::default().fg(if shuffle { theme.accent } else { Color::Reset }),
            ),
            Span::styled(" r ", key_style),
            Span::styled(
                format!(" {repeat_label}  "),
                Style::default().fg(if repeat_label != "Repeat Off" {
                    theme.accent
                } else {
                    Color::Reset
                }),
            ),
            Span::styled(" c ", key_style),
            Span::styled(
                format!(" Crossfade {crossfade_label}  "),
                Style::default().fg(if crossfade_label != "Off" {
                    theme.accent
                } else {
                    Color::Reset
                }),
            ),
            Span::styled(" f ", key_style),
            Span::raw(" Files  "),
        ]);
    }
    spans.extend([
        Span::styled(" t ", key_style),
        Span::raw(" Theme  "),
        Span::styled(" i ", key_style),
        Span::raw(" Track Info  "),
        Span::styled(" x ", key_style),
        Span::raw(" Clear Cache  "),
        Span::styled(" q ", key_style),
        Span::raw(" Quit"),
    ]);
    spans
}

/// Wrap spans into lines, breaking at group boundaries (every 2 spans = key + label).
fn wrap_lines(spans: Vec<Span<'static>>, inner_w: usize) -> Vec<Line<'static>> {
    if inner_w == 0 {
        return vec![Line::from(spans)];
    }
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut current_w: usize = 0;
    for chunk in spans.chunks(2) {
        let group_w: usize = Line::from(chunk.to_vec()).width();
        if current_w + group_w > inner_w && current_w > 0 {
            lines.push(Line::from(std::mem::take(&mut current)));
            current_w = 0;
        }
        current.extend(chunk.iter().cloned());
        current_w += group_w;
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

pub fn controls_height(
    width: u16,
    show_visualizer: bool,
    has_browser: bool,
    shuffle: bool,
    repeat_label: &str,
    crossfade_label: &str,
    theme: &Theme,
) -> u16 {
    let spans = build_control_spans(show_visualizer, has_browser, shuffle, repeat_label, crossfade_label, theme);
    let inner_w = width.saturating_sub(2) as usize;
    let lines = wrap_lines(spans, inner_w);
    lines.len() as u16 + 2 // +2 for borders
}

pub fn draw_controls(
    frame: &mut Frame,
    area: Rect,
    show_visualizer: bool,
    has_browser: bool,
    shuffle: bool,
    repeat_label: &str,
    crossfade_label: &str,
    theme: &Theme,
) {
    let spans = build_control_spans(show_visualizer, has_browser, shuffle, repeat_label, crossfade_label, theme);
    let inner_w = area.width.saturating_sub(2) as usize;
    let lines = wrap_lines(spans, inner_w);
    let help = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Controls "),
    );
    frame.render_widget(help, area);
}

pub fn draw_scope_hint(frame: &mut Frame, area: Rect, theme: &Theme) {
    let hint = Line::from(vec![
        Span::styled(" Run ", Style::default().fg(theme.dimmed)),
        Span::styled("cargo install scope-tui", Style::default().fg(theme.secondary)),
        Span::styled(" to enable audio visualizer", Style::default().fg(theme.dimmed)),
    ]);
    frame.render_widget(Paragraph::new(hint), area);
}
