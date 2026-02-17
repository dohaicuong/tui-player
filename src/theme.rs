use std::fs;

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::config_dir;

pub struct Theme {
    pub name: &'static str,
    pub accent: Color,
    pub secondary: Color,
    pub positive: Color,
    pub negative: Color,
    pub text: Color,
    pub dimmed: Color,
}

pub const THEMES: &[Theme] = &[
    Theme {
        name: "Default",
        accent: Color::Cyan,
        secondary: Color::Yellow,
        positive: Color::Green,
        negative: Color::Red,
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Dracula",
        accent: Color::Rgb(189, 147, 249),
        secondary: Color::Rgb(255, 121, 198),
        positive: Color::Rgb(80, 250, 123),
        negative: Color::Rgb(255, 85, 85),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Nord",
        accent: Color::Rgb(136, 192, 208),
        secondary: Color::Rgb(235, 203, 139),
        positive: Color::Rgb(163, 190, 140),
        negative: Color::Rgb(191, 97, 106),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Gruvbox",
        accent: Color::Rgb(214, 153, 62),
        secondary: Color::Rgb(250, 189, 47),
        positive: Color::Rgb(152, 151, 26),
        negative: Color::Rgb(204, 36, 29),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Rose Pine",
        accent: Color::Rgb(235, 188, 186),
        secondary: Color::Rgb(246, 193, 119),
        positive: Color::Rgb(156, 207, 216),
        negative: Color::Rgb(235, 111, 146),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Catppuccin",
        accent: Color::Rgb(203, 166, 247),
        secondary: Color::Rgb(249, 226, 175),
        positive: Color::Rgb(166, 227, 161),
        negative: Color::Rgb(243, 139, 168),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Tokyo Night",
        accent: Color::Rgb(122, 162, 247),
        secondary: Color::Rgb(224, 175, 104),
        positive: Color::Rgb(158, 206, 106),
        negative: Color::Rgb(247, 118, 142),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Solarized",
        accent: Color::Rgb(38, 139, 210),
        secondary: Color::Rgb(181, 137, 0),
        positive: Color::Rgb(133, 153, 0),
        negative: Color::Rgb(220, 50, 47),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Monokai",
        accent: Color::Rgb(102, 217, 239),
        secondary: Color::Rgb(230, 219, 116),
        positive: Color::Rgb(166, 226, 46),
        negative: Color::Rgb(249, 38, 114),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "One Dark",
        accent: Color::Rgb(97, 175, 239),
        secondary: Color::Rgb(229, 192, 123),
        positive: Color::Rgb(152, 195, 121),
        negative: Color::Rgb(224, 108, 117),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Kanagawa",
        accent: Color::Rgb(126, 156, 216),
        secondary: Color::Rgb(230, 195, 132),
        positive: Color::Rgb(152, 187, 108),
        negative: Color::Rgb(255, 93, 98),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Everforest",
        accent: Color::Rgb(127, 187, 179),
        secondary: Color::Rgb(219, 188, 127),
        positive: Color::Rgb(167, 192, 128),
        negative: Color::Rgb(230, 126, 128),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
    Theme {
        name: "Synthwave",
        accent: Color::Rgb(255, 126, 219),
        secondary: Color::Rgb(254, 222, 93),
        positive: Color::Rgb(114, 241, 184),
        negative: Color::Rgb(254, 68, 80),
        text: Color::White,
        dimmed: Color::DarkGray,
    },
];

pub fn load_theme() -> usize {
    fs::read_to_string(config_dir().join("theme"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|&i: &usize| i < THEMES.len())
        .unwrap_or(0)
}

pub fn save_theme(index: usize) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join("theme"), format!("{index}"));
}

pub fn draw_theme_selector(frame: &mut Frame, selected: usize) {
    let area = frame.area();
    // Each theme row: "  >> Name    ██ ██ ██ ██  " (~40 chars)
    let popup_w = 42u16.min(area.width);
    let popup_h = (THEMES.len() as u16 + 4).min(area.height); // +4 for borders + header + bottom
    let popup_x = area.width.saturating_sub(popup_w) / 2;
    let popup_y = area.height.saturating_sub(popup_h) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_w, popup_h);

    frame.render_widget(Clear, popup_area);

    let theme = &THEMES[selected];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent))
        .title(" Theme ")
        .title_bottom(" ↑/↓: Select  Enter: Apply  Esc: Close ");

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            theme.name,
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::raw(""));

    for (i, t) in THEMES.iter().enumerate() {
        let is_sel = i == selected;
        let marker = if is_sel { ">> " } else { "   " };

        let name_style = if is_sel {
            Style::default().fg(Color::Black).bg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text)
        };

        // Pad name to 14 chars for alignment
        let padded_name = format!("{:<14}", t.name);

        let mut spans = vec![
            Span::styled(marker, Style::default().fg(t.accent)),
            Span::styled(padded_name, name_style),
            Span::raw(" "),
            Span::styled("██", Style::default().fg(t.accent)),
            Span::raw(" "),
            Span::styled("██", Style::default().fg(t.secondary)),
            Span::raw(" "),
            Span::styled("██", Style::default().fg(t.positive)),
            Span::raw(" "),
            Span::styled("██", Style::default().fg(t.negative)),
        ];

        if is_sel {
            spans.push(Span::styled(" ◄", Style::default().fg(t.accent)));
        }

        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}
