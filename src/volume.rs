use ratatui::{
    layout::{Alignment, Rect},
    text::Line,
    widgets::{Block, BorderType, Borders},
    Frame,
};

use crate::gauge::RoundedGauge;
use crate::theme::Theme;

pub fn draw_volume(frame: &mut Frame, area: Rect, volume: f32, theme: &Theme) {
    let vol_pct = (volume * 100.0) as u16;
    let vol_ratio = (volume / 2.0) as f64;
    let vol_gauge = RoundedGauge::new(vol_ratio, String::new(), theme.positive)
        .overflow(0.5, theme.negative)
        .dimmed_color(theme.dimmed)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Volume ")
                .title(Line::from(format!(" {}% ", vol_pct)).alignment(Alignment::Right)),
        );
    frame.render_widget(vol_gauge, area);
}
