use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
    widgets::{Block, BorderType, Borders, Widget},
    Frame,
};
use rustfft::{num_complex::Complex, FftPlanner};

use crate::SampleBuf;

// Visualization modes
#[derive(Clone, Copy, PartialEq)]
pub enum VisMode {
    Oscilloscope,
    Vectorscope,
    Spectroscope,
}

impl VisMode {
    pub fn next(self) -> Self {
        match self {
            VisMode::Oscilloscope => VisMode::Vectorscope,
            VisMode::Vectorscope => VisMode::Spectroscope,
            VisMode::Spectroscope => VisMode::Oscilloscope,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VisMode::Oscilloscope => " Oscilloscope ",
            VisMode::Vectorscope => " Vectorscope ",
            VisMode::Spectroscope => " Spectroscope ",
        }
    }
}

// Braille dot positions per character cell (2 wide x 4 tall):
//   col0: bits 0,1,2,6  (top to bottom)
//   col1: bits 3,4,5,7  (top to bottom)
const BRAILLE_BASE: u32 = 0x2800;
const BRAILLE_DOTS: [[u8; 4]; 2] = [
    [0x01, 0x02, 0x04, 0x40], // left column
    [0x08, 0x10, 0x20, 0x80], // right column
];

struct OscilloscopeWidget<'a> {
    samples: &'a SampleBuf,
    channels: u16,
    block: Option<Block<'a>>,
}

impl<'a> OscilloscopeWidget<'a> {
    fn new(samples: &'a SampleBuf, channels: u16) -> Self {
        OscilloscopeWidget {
            samples,
            channels,
            block: None,
        }
    }

    fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl Widget for OscilloscopeWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = if let Some(block) = self.block {
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            area
        };

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let samples: Vec<f32> = if let Ok(s) = self.samples.lock() {
            s.iter().copied().collect()
        } else {
            return;
        };

        if samples.is_empty() {
            return;
        }

        let ch_count = self.channels.max(1) as usize;
        let px_w = inner.width as usize * 2;
        let px_h = inner.height as usize * 4;
        let mid_y = px_h as f32 / 2.0;

        let cols = inner.width as usize;
        let rows = inner.height as usize;
        let mut grid = vec![0u8; cols * rows];

        // Draw center reference line
        let center_py = px_h / 2;
        let center_cy = center_py / 4;
        let center_dy = center_py % 4;
        if center_cy < rows {
            for cx in 0..cols {
                grid[center_cy * cols + cx] |=
                    BRAILLE_DOTS[0][center_dy] | BRAILLE_DOTS[1][center_dy];
            }
        }
        let ref_grid = grid.clone();

        // Plot waveform (left channel)
        let total_mono = samples.len() / ch_count;
        for px_x in 0..px_w {
            let sample_idx = (px_x * total_mono) / px_w;
            let s = samples.get(sample_idx * ch_count).copied().unwrap_or(0.0);
            let py = ((1.0 - s.clamp(-1.0, 1.0)) * mid_y).min(px_h as f32 - 1.0) as usize;

            let cx = px_x / 2;
            let cy = py / 4;
            let dx = px_x % 2;
            let dy = py % 4;

            if cx < cols && cy < rows {
                grid[cy * cols + cx] |= BRAILLE_DOTS[dx][dy];
            }
        }

        for cy in 0..rows {
            for cx in 0..cols {
                let dots = grid[cy * cols + cx];
                let ch = char::from_u32(BRAILLE_BASE + dots as u32).unwrap_or(' ');
                let x = inner.x + cx as u16;
                let y = inner.y + cy as u16;
                let has_wave = (dots & !ref_grid[cy * cols + cx]) != 0;
                let color = if has_wave { Color::Green } else { Color::DarkGray };
                buf[(x, y)].set_char(ch).set_fg(color);
            }
        }
    }
}

struct VectorscopeWidget<'a> {
    samples: &'a SampleBuf,
    channels: u16,
    block: Option<Block<'a>>,
}

impl<'a> VectorscopeWidget<'a> {
    fn new(samples: &'a SampleBuf, channels: u16) -> Self {
        VectorscopeWidget {
            samples,
            channels,
            block: None,
        }
    }

    fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl Widget for VectorscopeWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = if let Some(block) = self.block {
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            area
        };

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let samples: Vec<f32> = if let Ok(s) = self.samples.lock() {
            s.iter().copied().collect()
        } else {
            return;
        };

        if samples.is_empty() {
            return;
        }

        let ch_count = self.channels.max(1) as usize;
        let px_w = inner.width as usize * 2;
        let px_h = inner.height as usize * 4;
        let mid_x = px_w as f32 / 2.0;
        let mid_y = px_h as f32 / 2.0;
        // Use the smaller dimension so the plot is square
        let radius = mid_x.min(mid_y);

        let cols = inner.width as usize;
        let rows = inner.height as usize;
        let mut grid = vec![0u8; cols * rows];

        // Draw crosshair reference lines (dimmed)
        // Vertical center line
        let center_px_x = px_w / 2;
        for py in 0..px_h {
            let cx = center_px_x / 2;
            let dx = center_px_x % 2;
            let cy = py / 4;
            let dy = py % 4;
            if cx < cols && cy < rows {
                grid[cy * cols + cx] |= BRAILLE_DOTS[dx][dy];
            }
        }
        // Horizontal center line
        let center_py = px_h / 2;
        for px_x in 0..px_w {
            let cx = px_x / 2;
            let dx = px_x % 2;
            let cy = center_py / 4;
            let dy = center_py % 4;
            if cx < cols && cy < rows {
                grid[cy * cols + cx] |= BRAILLE_DOTS[dx][dy];
            }
        }

        // Track which cells have crosshair bits for coloring
        let ref_grid = grid.clone();

        // Plot L/R sample pairs using mid/side rotation:
        //   X = (L - R) * 0.707  (side — stereo spread)
        //   Y = (L + R) * 0.707  (mid — mono content)
        // Mono = vertical line, stereo = wider spread
        let num_frames = samples.len() / ch_count;
        for i in 0..num_frames {
            let left = samples[i * ch_count].clamp(-1.0, 1.0);
            let right = if ch_count >= 2 {
                samples[i * ch_count + 1].clamp(-1.0, 1.0)
            } else {
                left
            };

            let side = (left - right) * 0.707;
            let mid = (left + right) * 0.707;

            let px_x = (mid_x + side * radius).clamp(0.0, px_w as f32 - 1.0) as usize;
            let py = (mid_y - mid * radius).clamp(0.0, px_h as f32 - 1.0) as usize;

            let cx = px_x / 2;
            let cy = py / 4;
            let dx = px_x % 2;
            let dy = py % 4;

            if cx < cols && cy < rows {
                grid[cy * cols + cx] |= BRAILLE_DOTS[dx][dy];
            }
        }

        // Render to buffer
        for cy in 0..rows {
            for cx in 0..cols {
                let dots = grid[cy * cols + cx];
                let ch = char::from_u32(BRAILLE_BASE + dots as u32).unwrap_or(' ');
                let x = inner.x + cx as u16;
                let y = inner.y + cy as u16;

                let has_wave = (dots & !ref_grid[cy * cols + cx]) != 0;

                let color = if has_wave {
                    Color::Green
                } else {
                    Color::DarkGray
                };

                buf[(x, y)].set_char(ch).set_fg(color);
            }
        }
    }
}

struct SpectroscopeWidget<'a> {
    samples: &'a SampleBuf,
    channels: u16,
    block: Option<Block<'a>>,
}

impl<'a> SpectroscopeWidget<'a> {
    fn new(samples: &'a SampleBuf, channels: u16) -> Self {
        SpectroscopeWidget {
            samples,
            channels,
            block: None,
        }
    }

    fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }
}

impl Widget for SpectroscopeWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let inner = if let Some(block) = self.block {
            let inner = block.inner(area);
            block.render(area, buf);
            inner
        } else {
            area
        };

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let samples: Vec<f32> = if let Ok(s) = self.samples.lock() {
            s.iter().copied().collect()
        } else {
            return;
        };

        if samples.is_empty() {
            return;
        }

        let ch_count = self.channels.max(1) as usize;
        let px_h = inner.height as usize * 4;
        let cols = inner.width as usize;
        let rows = inner.height as usize;

        // Mix down to mono
        let num_frames = samples.len() / ch_count;
        let mut mono: Vec<f32> = Vec::with_capacity(num_frames);
        for i in 0..num_frames {
            let mut sum = 0.0;
            for c in 0..ch_count {
                sum += samples[i * ch_count + c];
            }
            mono.push(sum / ch_count as f32);
        }

        // FFT — use power-of-2 window
        let fft_size = mono.len().next_power_of_two().max(64);
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        let mut fft_input: Vec<Complex<f32>> = Vec::with_capacity(fft_size);
        // Apply Hann window
        let window_len = mono.len().min(fft_size);
        for i in 0..window_len {
            let w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (window_len as f32 - 1.0)).cos());
            fft_input.push(Complex::new(mono[mono.len() - window_len + i] * w, 0.0));
        }
        // Zero-pad remainder
        fft_input.resize(fft_size, Complex::new(0.0, 0.0));

        fft.process(&mut fft_input);

        // Only use first half (positive frequencies)
        let num_bins = fft_size / 2;
        let magnitudes: Vec<f32> = fft_input[..num_bins]
            .iter()
            .map(|c| c.norm() / fft_size as f32)
            .collect();

        // Map bins to columns using logarithmic scale
        let mut col_mags = vec![0.0f32; cols];
        if num_bins > 1 {
            for col in 0..cols {
                // Log scale: map column to frequency bin
                let frac = col as f32 / cols as f32;
                let bin_f = (num_bins as f32).powf(frac);
                let bin = (bin_f as usize).clamp(1, num_bins - 1);
                // Average nearby bins for smoother result
                let lo = bin.saturating_sub(1);
                let hi = (bin + 1).min(num_bins - 1);
                let mut sum = 0.0;
                let mut count = 0;
                for b in lo..=hi {
                    sum += magnitudes[b];
                    count += 1;
                }
                col_mags[col] = sum / count as f32;
            }
        }

        // Normalize magnitudes
        let max_mag = col_mags.iter().cloned().fold(0.0f32, f32::max).max(0.001);

        // Render using braille — each column bar grows upward from bottom
        let mut grid = vec![0u8; cols * rows];

        for col in 0..cols {
            let height = (col_mags[col] / max_mag * px_h as f32).round() as usize;
            let height = height.min(px_h);

            // Fill from bottom up
            for py in (px_h - height)..px_h {
                let cx = col; // one braille column (left dot) per screen column
                let cy = py / 4;
                let dy = py % 4;
                if cy < rows {
                    grid[cy * cols + cx] |= BRAILLE_DOTS[0][dy] | BRAILLE_DOTS[1][dy];
                }
            }
        }

        for cy in 0..rows {
            for cx in 0..cols {
                let dots = grid[cy * cols + cx];
                let ch = char::from_u32(BRAILLE_BASE + dots as u32).unwrap_or(' ');
                let x = inner.x + cx as u16;
                let y = inner.y + cy as u16;

                let color = if dots != 0 {
                    // Color gradient based on vertical position
                    let frac = cy as f32 / rows as f32;
                    if frac < 0.33 {
                        Color::Red
                    } else if frac < 0.66 {
                        Color::Yellow
                    } else {
                        Color::Green
                    }
                } else {
                    Color::DarkGray
                };

                buf[(x, y)].set_char(ch).set_fg(color);
            }
        }
    }
}

/// Render the active visualizer widget into the given area.
pub fn draw_visualizer(frame: &mut Frame, area: Rect, mode: VisMode, samples: &SampleBuf, channels: u16) {
    let vis_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(mode.label());
    match mode {
        VisMode::Oscilloscope => {
            let w = OscilloscopeWidget::new(samples, channels).block(vis_block);
            frame.render_widget(w, area);
        }
        VisMode::Vectorscope => {
            let w = VectorscopeWidget::new(samples, channels).block(vis_block);
            frame.render_widget(w, area);
        }
        VisMode::Spectroscope => {
            let w = SpectroscopeWidget::new(samples, channels).block(vis_block);
            frame.render_widget(w, area);
        }
    }
}
