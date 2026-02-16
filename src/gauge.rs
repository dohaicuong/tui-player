use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
    widgets::{Block, Widget},
};

pub struct RoundedGauge<'a> {
    ratio: f64,
    label: String,
    filled_color: Color,
    overflow_at: Option<f64>,
    overflow_color: Color,
    block: Option<Block<'a>>,
}

impl<'a> RoundedGauge<'a> {
    pub fn new(ratio: f64, label: String, filled_color: Color) -> Self {
        RoundedGauge {
            ratio: ratio.clamp(0.0, 1.0),
            label,
            filled_color,
            overflow_at: None,
            overflow_color: Color::Red,
            block: None,
        }
    }

    pub fn overflow(mut self, threshold: f64, color: Color) -> Self {
        self.overflow_at = Some(threshold);
        self.overflow_color = color;
        self
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl Widget for RoundedGauge<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = if let Some(block) = self.block {
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            area
        };

        if inner.width < 2 || inner.height == 0 {
            return;
        }

        let width = inner.width as usize;
        let filled = (self.ratio * width as f64).round() as usize;
        let overflow_col = self
            .overflow_at
            .map(|t| (t * width as f64).round() as usize)
            .unwrap_or(width);
        let y = inner.y;

        for col in 0..width {
            let x = inner.x + col as u16;
            let fill_color = if col >= overflow_col {
                self.overflow_color
            } else {
                self.filled_color
            };
            let (ch, fg, bg) = if filled == 0 {
                if col == 0 {
                    ('╶', Color::DarkGray, Color::Reset)
                } else if col == width - 1 {
                    ('╴', Color::DarkGray, Color::Reset)
                } else {
                    ('─', Color::DarkGray, Color::Reset)
                }
            } else if col < filled {
                if col == 0 {
                    ('╺', fill_color, Color::Reset)
                } else if col == filled - 1 && filled < width {
                    ('╸', fill_color, Color::Reset)
                } else {
                    ('━', fill_color, Color::Reset)
                }
            } else {
                if col == width - 1 {
                    ('╴', Color::DarkGray, Color::Reset)
                } else {
                    ('─', Color::DarkGray, Color::Reset)
                }
            };

            buf[(x, y)].set_char(ch).set_fg(fg).set_bg(bg);
        }

        let label_len = self.label.len();
        if label_len <= width {
            let start = inner.x + (width - label_len) as u16 / 2;
            for (i, ch) in self.label.chars().enumerate() {
                let x = start + i as u16;
                let col = (x - inner.x) as usize;
                let fg = if col < filled {
                    Color::White
                } else {
                    Color::Gray
                };
                buf[(x, y)].set_char(ch).set_fg(fg).set_bg(Color::Reset);
            }
        }
    }
}
