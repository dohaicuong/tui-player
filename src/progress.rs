use std::time::Duration;

use ratatui::{
    layout::{Alignment, Rect},
    text::Line,
    widgets::{Block, BorderType, Borders},
    Frame,
};

use crate::gauge::RoundedGauge;
use crate::theme::Theme;

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    format!("{}:{:02}", secs / 60, secs % 60)
}

pub fn draw_progress(
    frame: &mut Frame,
    area: Rect,
    elapsed: Duration,
    total: Option<Duration>,
    waveform: Option<&[f32]>,
    theme: &Theme,
) {
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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Progress ")
        .title(Line::from(format!(" {progress_label} ")).alignment(Alignment::Right));

    let mut gauge = RoundedGauge::new(ratio, String::new(), theme.accent)
        .dimmed_color(theme.dimmed)
        .block(block);
    if let Some(wf) = waveform {
        gauge = gauge.waveform(wf);
    }
    frame.render_widget(gauge, area);
}
