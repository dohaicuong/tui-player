use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

fn build_control_spans(
    show_visualizer: bool,
    has_browser: bool,
    shuffle: bool,
    repeat_label: &str,
    crossfade_label: &str,
) -> Vec<Span<'static>> {
    let mut spans = vec![
        Span::styled(" Space ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Play/Pause  "),
        Span::styled(" ←/→ ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Seek ±5s  "),
        Span::styled(" ↑/↓ ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Volume  "),
    ];
    if show_visualizer {
        spans.extend([
            Span::styled(" v ", Style::default().fg(Color::Black).bg(Color::Yellow)),
            Span::raw(" Vis Mode  "),
        ]);
    }
    spans.extend([
        Span::styled(" l ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Lyrics  "),
    ]);
    spans.extend([
        Span::styled(" e ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" EQ  "),
    ]);
    if has_browser {
        spans.extend([
            Span::styled(" n/N ", Style::default().fg(Color::Black).bg(Color::Yellow)),
            Span::raw(" Next/Prev  "),
            Span::styled(" s ", Style::default().fg(Color::Black).bg(Color::Yellow)),
            Span::styled(
                if shuffle { " Shuffle On  " } else { " Shuffle Off  " },
                Style::default().fg(if shuffle { Color::Cyan } else { Color::Reset }),
            ),
            Span::styled(" r ", Style::default().fg(Color::Black).bg(Color::Yellow)),
            Span::styled(
                format!(" {repeat_label}  "),
                Style::default().fg(if repeat_label != "Repeat Off" {
                    Color::Cyan
                } else {
                    Color::Reset
                }),
            ),
            Span::styled(" c ", Style::default().fg(Color::Black).bg(Color::Yellow)),
            Span::styled(
                format!(" Crossfade {crossfade_label}  "),
                Style::default().fg(if crossfade_label != "Off" {
                    Color::Cyan
                } else {
                    Color::Reset
                }),
            ),
            Span::styled(" f ", Style::default().fg(Color::Black).bg(Color::Yellow)),
            Span::raw(" Files  "),
        ]);
    }
    spans.extend([
        Span::styled(" x ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Clear Cache  "),
        Span::styled(" q ", Style::default().fg(Color::Black).bg(Color::Yellow)),
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
) -> u16 {
    let spans = build_control_spans(show_visualizer, has_browser, shuffle, repeat_label, crossfade_label);
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
) {
    let spans = build_control_spans(show_visualizer, has_browser, shuffle, repeat_label, crossfade_label);
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

pub fn draw_scope_hint(frame: &mut Frame, area: Rect) {
    let hint = Line::from(vec![
        Span::styled(" Run ", Style::default().fg(Color::DarkGray)),
        Span::styled("cargo install scope-tui", Style::default().fg(Color::Yellow)),
        Span::styled(" to enable audio visualizer", Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(hint), area);
}
