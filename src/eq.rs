use std::sync::{Arc, Mutex};

use biquad::{Biquad, Coefficients, DirectForm2Transposed, ToHertz, Type};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::theme::Theme;

pub const NUM_BANDS: usize = 32;

// 1/3-octave ISO center frequencies
pub const BAND_FREQS: [f32; NUM_BANDS] = [
    16.0, 20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0,
    400.0, 500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0,
    6300.0, 8000.0, 10000.0, 12500.0, 16000.0, 20000.0,
];

const MAX_GAIN: f32 = 12.0;
const EQ_Q: f32 = 4.3; // 1/3-octave bandwidth

pub struct EqParams {
    pub enabled: bool,
    pub gains: [f32; NUM_BANDS],
    pub preset_index: usize,
}

impl Default for EqParams {
    fn default() -> Self {
        EqParams {
            enabled: true,
            gains: [0.0; NUM_BANDS],
            preset_index: 0,
        }
    }
}

pub type SharedEqParams = Arc<Mutex<EqParams>>;

#[rustfmt::skip]
pub const PRESETS: &[(&str, [f32; NUM_BANDS])] = &[
    ("Flat", [0.0; NUM_BANDS]),
    ("Rock", [
        3.0, 3.0, 3.0, 4.0, 4.0, 3.0, 2.0, 1.0,
        0.0, 0.0,-1.0,-1.0,-1.0, 0.0, 0.0, 0.0,
        1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 3.0, 4.0,
        4.0, 4.0, 3.0, 3.0, 3.0, 2.0, 2.0, 1.0,
    ]),
    ("Pop", [
       -1.0,-1.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0,
        4.0, 4.0, 3.0, 3.0, 2.0, 2.0, 1.0, 0.0,
       -1.0,-1.0,-1.0,-1.0, 0.0, 0.0, 0.0, 0.0,
        1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 0.0,
    ]),
    ("Jazz", [
        3.0, 3.0, 3.0, 2.0, 2.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 2.0, 2.0, 1.0, 0.0,-1.0,
       -2.0,-2.0,-1.0, 0.0, 0.0, 1.0, 2.0, 2.0,
        3.0, 3.0, 3.0, 3.0, 4.0, 4.0, 3.0, 3.0,
    ]),
    ("Classical", [
        4.0, 4.0, 3.0, 3.0, 3.0, 2.0, 2.0, 1.0,
        1.0, 0.0, 0.0,-1.0,-1.0,-1.0,-1.0, 0.0,
        0.0, 0.0, 0.0, 0.0,-1.0,-1.0, 0.0, 0.0,
        1.0, 2.0, 2.0, 3.0, 3.0, 3.0, 4.0, 4.0,
    ]),
    ("Bass Boost", [
        8.0, 8.0, 7.0, 7.0, 6.0, 6.0, 5.0, 5.0,
        4.0, 4.0, 3.0, 2.0, 1.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    ]),
    ("Treble Boost", [
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 3.0, 4.0,
        5.0, 5.0, 6.0, 6.0, 7.0, 7.0, 8.0, 8.0,
    ]),
    ("Vocal", [
       -2.0,-2.0,-2.0,-1.0,-1.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 4.0,
        5.0, 5.0, 5.0, 4.0, 3.0, 3.0, 2.0, 1.0,
        0.0, 0.0, 0.0, 0.0,-1.0,-1.0,-2.0,-2.0,
    ]),
    ("Melodic Death", [
        4.0, 4.0, 5.0, 5.0, 6.0, 5.0, 4.0, 3.0,
        2.0, 1.0, 0.0,-1.0,-2.0,-2.0,-1.0, 0.0,
        1.0, 2.0, 3.0, 4.0, 5.0, 5.0, 4.0, 3.0,
        3.0, 4.0, 5.0, 5.0, 4.0, 3.0, 2.0, 1.0,
    ]),
    ("Heavy Metal", [
        5.0, 5.0, 6.0, 6.0, 5.0, 4.0, 3.0, 2.0,
        1.0, 0.0,-1.0,-2.0,-3.0,-3.0,-2.0,-1.0,
        0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 5.0, 5.0,
        6.0, 6.0, 5.0, 5.0, 4.0, 3.0, 2.0, 1.0,
    ]),
    ("Power Metal", [
        3.0, 3.0, 4.0, 5.0, 5.0, 4.0, 3.0, 2.0,
        1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0,
        3.0, 3.0, 4.0, 5.0, 5.0, 5.0, 4.0, 4.0,
        5.0, 5.0, 6.0, 6.0, 5.0, 4.0, 3.0, 2.0,
    ]),
];

fn make_filter(freq: f32, gain_db: f32, sample_rate: f32) -> DirectForm2Transposed<f32> {
    let max_freq = sample_rate / 2.0 - 1.0;
    let clamped_freq = freq.min(max_freq).max(1.0);
    let coeffs = Coefficients::<f32>::from_params(
        Type::PeakingEQ(gain_db),
        sample_rate.hz(),
        clamped_freq.hz(),
        EQ_Q,
    )
    .unwrap_or_else(|_| {
        Coefficients::<f32>::from_params(
            Type::PeakingEQ(0.0),
            sample_rate.hz(),
            clamped_freq.hz(),
            EQ_Q,
        )
        .unwrap()
    });
    DirectForm2Transposed::<f32>::new(coeffs)
}

pub struct EqFilters {
    /// filters[channel][band]
    filters: Vec<[DirectForm2Transposed<f32>; NUM_BANDS]>,
    cached_gains: [f32; NUM_BANDS],
    cached_enabled: bool,
    sample_rate: f32,
}

impl EqFilters {
    pub fn new(channels: u16, sample_rate: f32, params: &EqParams) -> Self {
        let filters: Vec<_> = (0..channels as usize)
            .map(|_| {
                std::array::from_fn(|i| make_filter(BAND_FREQS[i], params.gains[i], sample_rate))
            })
            .collect();
        EqFilters {
            filters,
            cached_gains: params.gains,
            cached_enabled: params.enabled,
            sample_rate,
        }
    }

    pub fn process(&mut self, sample: f32, channel: usize) -> f32 {
        if !self.cached_enabled {
            return sample;
        }
        let ch_filters = &mut self.filters[channel];
        let mut out = sample;
        for filter in ch_filters.iter_mut() {
            out = filter.run(out);
        }
        out
    }

    pub fn update_if_changed(&mut self, params: &EqParams) {
        if params.enabled == self.cached_enabled && params.gains == self.cached_gains {
            return;
        }
        self.cached_enabled = params.enabled;
        if params.gains != self.cached_gains {
            self.cached_gains = params.gains;
            for ch_filters in &mut self.filters {
                for (i, filter) in ch_filters.iter_mut().enumerate() {
                    *filter =
                        make_filter(BAND_FREQS[i], self.cached_gains[i], self.sample_rate);
                }
            }
        }
    }
}

// --- Config persistence ---

fn config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    std::path::PathBuf::from(home)
        .join(".config")
        .join("tui-player")
        .join("eq")
}

pub fn load_eq() -> EqParams {
    let content = match std::fs::read_to_string(config_path()) {
        Ok(c) => c,
        Err(_) => return EqParams::default(),
    };
    let mut lines = content.lines();
    let enabled = lines.next().map(|s| s.trim() == "true").unwrap_or(true);
    let preset_index = lines
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let gains_line = lines.next().unwrap_or("");
    let mut gains = [0.0f32; NUM_BANDS];
    for (i, val) in gains_line.split(',').enumerate() {
        if i >= NUM_BANDS {
            break;
        }
        if let Ok(g) = val.trim().parse::<f32>() {
            gains[i] = g.clamp(-MAX_GAIN, MAX_GAIN);
        }
    }
    EqParams {
        enabled,
        gains,
        preset_index,
    }
}

pub fn save_eq(params: &EqParams) {
    let dir = config_path().parent().unwrap().to_path_buf();
    let _ = std::fs::create_dir_all(&dir);
    let gains_str: Vec<String> = params.gains.iter().map(|g| format!("{g}")).collect();
    let content = format!(
        "{}\n{}\n{}",
        if params.enabled { "true" } else { "false" },
        params.preset_index,
        gains_str.join(",")
    );
    let _ = std::fs::write(config_path(), content);
}

// --- Drawing ---

fn format_freq(f: f32) -> String {
    if f >= 1000.0 {
        let k = f / 1000.0;
        if k == k.floor() {
            format!("{}k", k as u32)
        } else {
            format!("{:.1}k", k)
        }
    } else {
        if f == f.floor() {
            format!("{}", f as u32)
        } else {
            format!("{:.1}", f)
        }
    }
}

pub fn draw_eq(frame: &mut Frame, params: &EqParams, selected_band: usize, hover_band: Option<usize>, theme: &Theme) -> Rect {
    let area = frame.area();
    // 32 bars × 2 chars = 64, + 1 leading + 4 dB label + 2 border = 71
    let popup_width = 74u16.min(area.width);
    let popup_height = 22u16.min(area.height);
    let popup_x = area.width.saturating_sub(popup_width) / 2;
    let popup_y = area.height.saturating_sub(popup_height) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let preset_name = PRESETS
        .get(params.preset_index)
        .map(|(name, _)| *name)
        .unwrap_or("Custom");

    let status = if params.enabled { "ON" } else { "OFF" };
    let sel_freq = format_freq(BAND_FREQS[selected_band]);
    let sel_gain = params.gains[selected_band];
    let sel_gain_str = if sel_gain >= 0.0 {
        format!("+{:.0}", sel_gain)
    } else {
        format!("{:.0}", sel_gain)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(format!(" Equalizer [{status}] "))
        .title_bottom(Line::from(
            " ←/→: Band  ↑/↓: Gain  p: Preset  0: Flat  s: Toggle ",
        ));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    if inner.height < 6 || inner.width < 40 {
        return inner;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Header: preset + selected band info
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            preset_name,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("    "),
        Span::styled(
            format!("▸ {sel_freq} Hz  {sel_gain_str} dB"),
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::raw(""));

    // Bar area
    let bar_height = inner.height.saturating_sub(5) as usize;
    if bar_height == 0 {
        return inner;
    }
    let zero_row = bar_height / 2;

    for row in 0..bar_height {
        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::raw(" "));

        let db_at_row =
            MAX_GAIN - (row as f32 / (bar_height - 1).max(1) as f32) * 2.0 * MAX_GAIN;

        for (band, gain) in params.gains.iter().enumerate() {
            let is_selected = band == selected_band;

            let filled = if *gain >= 0.0 {
                db_at_row >= 0.0 && db_at_row <= *gain
            } else {
                db_at_row <= 0.0 && db_at_row >= *gain
            };

            let is_zero_line = row == zero_row;

            let (ch, style) = if filled {
                let color = if is_selected {
                    theme.accent
                } else if *gain >= 0.0 {
                    theme.positive
                } else {
                    theme.negative
                };
                ("██", Style::default().fg(color))
            } else if is_zero_line {
                let color = if is_selected {
                    theme.accent
                } else {
                    theme.dimmed
                };
                ("──", Style::default().fg(color))
            } else if is_selected {
                ("▏▕", Style::default().fg(theme.dimmed))
            } else {
                ("  ", Style::default())
            };

            spans.push(Span::styled(ch, style));
        }

        // dB markers on right
        if row == 0 {
            spans.push(Span::styled(
                format!(" +{:.0}", MAX_GAIN),
                Style::default().fg(theme.dimmed),
            ));
        } else if row == zero_row {
            spans.push(Span::styled("  0", Style::default().fg(theme.dimmed)));
        } else if row == bar_height - 1 {
            spans.push(Span::styled(
                format!(" -{:.0}", MAX_GAIN),
                Style::default().fg(theme.dimmed),
            ));
        }

        lines.push(Line::from(spans));
    }

    // Frequency axis — sparse labels at key positions
    // Build a char buffer the width of the bar area (64 chars) and place labels
    let bar_width = NUM_BANDS * 2;
    let mut axis = vec![' '; bar_width];

    // Label positions: show ~8 evenly spaced labels
    let label_bands: &[usize] = &[0, 4, 8, 12, 16, 21, 25, 28, 31];
    for &b in label_bands {
        let label = format_freq(BAND_FREQS[b]);
        let col = b * 2;
        for (j, ch) in label.chars().enumerate() {
            if col + j < bar_width {
                axis[col + j] = ch;
            }
        }
    }

    let axis_str: String = axis.into_iter().collect();
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(axis_str, Style::default().fg(theme.secondary)),
        Span::styled(" Hz", Style::default().fg(theme.dimmed)),
    ]));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);

    // Hover tooltip on top border
    if let Some(hb) = hover_band {
        if hb < NUM_BANDS {
            let freq = format_freq(BAND_FREQS[hb]);
            let gain = params.gains[hb];
            let gain_str = if gain >= 0.0 {
                format!("+{:.0}", gain)
            } else {
                format!("{:.0}", gain)
            };
            let label = format!(" {} Hz  {} dB ", freq, gain_str);
            let label_len = label.len() as u16;
            let band_x = inner.x + 1 + hb as u16 * 2;
            let start_x = band_x
                .saturating_sub(label_len / 2)
                .max(popup_area.x)
                .min(popup_area.x + popup_area.width.saturating_sub(label_len));
            let hover_rect = Rect::new(start_x, popup_area.y, label_len, 1);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    label,
                    Style::default().fg(theme.secondary),
                )),
                hover_rect,
            );
        }
    }

    inner
}
