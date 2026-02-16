use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

pub fn draw_controls(frame: &mut Frame, area: Rect, show_visualizer: bool) {
    let mut help_spans = vec![
        Span::styled(" Space ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Play/Pause  "),
        Span::styled(" ←/→ ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Seek ±5s  "),
        Span::styled(" ↑/↓ ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::raw(" Volume  "),
    ];
    if show_visualizer {
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
    let help = Paragraph::new(Line::from(help_spans)).block(
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
