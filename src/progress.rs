use std::time::Duration;

use ratatui::{
    layout::Rect,
    style::Color,
    widgets::{Block, BorderType, Borders},
    Frame,
};

use crate::gauge::RoundedGauge;

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

pub fn draw_progress(frame: &mut Frame, area: Rect, elapsed: Duration, total: Option<Duration>) {
    let progress_label = match total {
        Some(t) if !t.is_zero() => {
            format!("{} / {}", format_duration(elapsed), format_duration(t))
        }
        _ => format_duration(elapsed),
    };
    let ratio = total
        .map(|t| {
            if t.is_zero() {
                0.0
            } else {
                (elapsed.as_secs_f64() / t.as_secs_f64()).min(1.0)
            }
        })
        .unwrap_or(0.0);
    let gauge = RoundedGauge::new(ratio, progress_label, Color::Cyan).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Progress "),
    );
    frame.render_widget(gauge, area);
}
