use ratatui::{
    layout::{Alignment, Rect},
    style::Color,
    text::Line,
    widgets::{Block, BorderType, Borders},
    Frame,
};

use crate::gauge::RoundedGauge;

pub fn draw_volume(frame: &mut Frame, area: Rect, volume: f32) {
    let vol_pct = (volume * 100.0) as u16;
    let vol_ratio = (volume / 2.0) as f64;
    let vol_gauge = RoundedGauge::new(vol_ratio, String::new(), Color::Green)
        .overflow(0.5, Color::Red)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Volume ")
                .title(Line::from(format!(" {}% ", vol_pct)).alignment(Alignment::Right)),
        );
    frame.render_widget(vol_gauge, area);
}
