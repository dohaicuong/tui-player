use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
    widgets::{Block, Widget},
};

const WAVEFORM_BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub struct RoundedGauge<'a> {
    ratio: f64,
    label: String,
    filled_color: Color,
    overflow_at: Option<f64>,
    overflow_color: Color,
    block: Option<Block<'a>>,
    waveform: Option<&'a [f32]>,
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
            waveform: None,
        }
    }

    pub fn waveform(mut self, wf: &'a [f32]) -> Self {
        self.waveform = Some(wf);
        self
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

        if let Some(wf) = self.waveform {
            let wf_len = wf.len();
            for col in 0..width {
                let x = inner.x + col as u16;
                let start = col * wf_len / width;
                let end = ((col + 1) * wf_len / width).min(wf_len);
                let amp = if start < end {
                    wf[start..end].iter().cloned().fold(0.0f32, f32::max)
                } else {
                    wf.get(start).copied().unwrap_or(0.0)
                };
                let block_idx = if amp > 0.0 {
                    ((amp * 7.0).round() as usize + 1).min(8)
                } else {
                    0
                };
                let ch = WAVEFORM_BLOCKS[block_idx];
                let fill_color = if col >= overflow_col {
                    self.overflow_color
                } else {
                    self.filled_color
                };
                let fg = if col < filled { fill_color } else { Color::DarkGray };
                buf[(x, y)].set_char(ch).set_fg(fg).set_bg(Color::Reset);
            }
        } else {
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
