//! Colour depth: 24-bit when the terminal can draw it, otherwise every RGB
//! colour is mapped to the nearest xterm-256 entry.
//!
//! The conversion happens once, at output: [`ColorBackend`] wraps the real
//! terminal backend and rewrites `Color::Rgb` in the cells it is asked to
//! draw. Doing it there (rather than in `ColorDef::to_color`) keeps every
//! colour computation in the app (blending, luminance, the GUI projection) in
//! exact RGB, and it also covers RGB colours that do not come from the theme
//! (syntax highlighting, mirrored agent panes).

use std::collections::HashMap;
use std::io;
use std::sync::OnceLock;

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::Color;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::terminal::{TermEnv, truecolor_support};

/// The `color_mode` config key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// Detect from the environment (`COLORTERM`, known terminals).
    #[default]
    Auto,
    TrueColor,
    Ansi256,
}

impl ColorMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ColorMode::Auto => "auto",
            ColorMode::TrueColor => "truecolor",
            ColorMode::Ansi256 => "256",
        }
    }

    /// Lenient parse: unknown values fall back to `auto` so a typo never
    /// makes the whole config file unreadable.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" | "24-bit" | "rgb" => ColorMode::TrueColor,
            "256" | "ansi256" | "xterm-256" | "8bit" => ColorMode::Ansi256,
            _ => ColorMode::Auto,
        }
    }
}

impl Serialize for ColorMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ColorMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(match value {
            serde_json::Value::String(s) => ColorMode::parse(&s),
            serde_json::Value::Number(n) if n.as_u64() == Some(256) => ColorMode::Ansi256,
            _ => ColorMode::Auto,
        })
    }
}

/// What AIW actually emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Ansi256,
}

impl ColorDepth {
    pub fn label(self) -> &'static str {
        match self {
            ColorDepth::TrueColor => "24-bit (truecolor)",
            ColorDepth::Ansi256 => "256 colours",
        }
    }
}

/// Resolve the configured mode against the environment; also says why.
pub fn resolve_depth(mode: ColorMode, env: &TermEnv) -> (ColorDepth, String) {
    match mode {
        ColorMode::TrueColor => (ColorDepth::TrueColor, "color_mode = truecolor".to_string()),
        ColorMode::Ansi256 => (ColorDepth::Ansi256, "color_mode = 256".to_string()),
        ColorMode::Auto => {
            let (truecolor, why) = truecolor_support(env);
            let depth = if truecolor {
                ColorDepth::TrueColor
            } else {
                ColorDepth::Ansi256
            };
            (depth, why)
        }
    }
}

/// RGB for ratatui's named / indexed colours using the stock xterm palette
/// (the user's real terminal palette is unknowable). `Reset` has none.
pub fn named_rgb(color: Color) -> Option<(u8, u8, u8)> {
    let index = match color {
        Color::Rgb(r, g, b) => return Some((r, g, b)),
        Color::Reset => return None,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        Color::Indexed(i) => i,
    };
    Some(xterm_rgb(index))
}

const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The stock xterm RGB value of a 256-colour palette index.
pub fn xterm_rgb(index: u8) -> (u8, u8, u8) {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let i = index - 16;
            (
                CUBE_LEVELS[(i / 36) as usize],
                CUBE_LEVELS[((i / 6) % 6) as usize],
                CUBE_LEVELS[(i % 6) as usize],
            )
        }
        _ => {
            let level = 8 + 10 * (index - 232);
            (level, level, level)
        }
    }
}

/// CIE L*a*b* (D65) of an sRGB colour, for perceptual distance.
fn lab((r, g, b): (u8, u8, u8)) -> [f64; 3] {
    fn linear(c: u8) -> f64 {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f64| {
        if t > 0.008856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// Lab values of palette entries 16..=255. The 16 ANSI colours are skipped on
/// purpose: users recolour them, so they would not look like the theme.
fn palette_lab() -> &'static [[f64; 3]; 240] {
    static PALETTE: OnceLock<[[f64; 3]; 240]> = OnceLock::new();
    PALETTE.get_or_init(|| std::array::from_fn(|i| lab(xterm_rgb(i as u8 + 16))))
}

/// Nearest xterm-256 index (16..=255: 6×6×6 cube or grey ramp) by CIE76
/// distance in L*a*b*.
pub fn rgb_to_ansi256(rgb: (u8, u8, u8)) -> u8 {
    let target = lab(rgb);
    let mut best = (f64::MAX, 16u8);
    for (i, candidate) in palette_lab().iter().enumerate() {
        let d = (0..3)
            .map(|k| (candidate[k] - target[k]).powi(2))
            .sum::<f64>();
        if d < best.0 {
            best = (d, i as u8 + 16);
        }
    }
    best.1
}

/// Backend wrapper that downgrades RGB cells to 256 colours when needed.
/// Everything else is passed straight through.
pub struct ColorBackend<B> {
    inner: B,
    depth: ColorDepth,
    cache: HashMap<(u8, u8, u8), u8>,
}

impl<B> ColorBackend<B> {
    pub fn new(inner: B, depth: ColorDepth) -> Self {
        Self {
            inner,
            depth,
            cache: HashMap::new(),
        }
    }

    fn map(&mut self, color: Color) -> Color {
        match color {
            Color::Rgb(r, g, b) => Color::Indexed(
                *self
                    .cache
                    .entry((r, g, b))
                    .or_insert_with(|| rgb_to_ansi256((r, g, b))),
            ),
            other => other,
        }
    }
}

impl<B: Backend> Backend for ColorBackend<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), B::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        if self.depth == ColorDepth::TrueColor {
            return self.inner.draw(content);
        }
        let cells: Vec<(u16, u16, Cell)> = content
            .map(|(x, y, cell)| {
                let mut cell = cell.clone();
                cell.fg = self.map(cell.fg);
                cell.bg = self.map(cell.bg);
                cell.underline_color = self.map(cell.underline_color);
                (x, y, cell)
            })
            .collect();
        self.inner
            .draw(cells.iter().map(|(x, y, cell)| (*x, *y, cell)))
    }

    fn append_lines(&mut self, n: u16) -> Result<(), B::Error> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> Result<(), B::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), B::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, B::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), B::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), B::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), B::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, B::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, B::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), B::Error> {
        Backend::flush(&mut self.inner)
    }
}

impl<B: io::Write> io::Write for ColorBackend<B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Theme, ThemeName};
    use ratatui::backend::TestBackend;

    #[test]
    fn exact_palette_colours_map_to_themselves() {
        for index in 16..=255u8 {
            let rgb = xterm_rgb(index);
            let mapped = rgb_to_ansi256(rgb);
            // Cube greys duplicate ramp greys (e.g. 16 == black); any exact
            // twin is fine.
            assert_eq!(xterm_rgb(mapped), rgb, "index {index}");
        }
    }

    #[test]
    fn near_greys_use_the_grey_ramp_and_hues_stay_hues() {
        // Ember's background is a warm near-black.
        assert_eq!(rgb_to_ansi256((0x14, 0x11, 0x10)), 233);
        assert_eq!(rgb_to_ansi256((0x80, 0x80, 0x80)), 244);
        let blue = xterm_rgb(rgb_to_ansi256((0x72, 0xa7, 0xf5)));
        assert!(blue.2 > blue.0 && blue.2 > blue.1, "{blue:?}");
        let amber = xterm_rgb(rgb_to_ansi256((0xf0, 0xa6, 0x40)));
        assert!(amber.0 > amber.1 && amber.1 > amber.2, "{amber:?}");
    }

    /// In 256-colour mode the quiet chrome must not collapse: background,
    /// selected row and border stay three different colours, and the status
    /// colours stay apart from each other.
    #[test]
    fn acc_themes_keep_their_structure_in_256_colours() {
        for name in [
            ThemeName::Ember,
            ThemeName::Graphite,
            ThemeName::Midnight,
            ThemeName::Paper,
            ThemeName::Porcelain,
        ] {
            let t = Theme::load(&name);
            let idx = |c: &crate::theme::ColorDef| rgb_to_ansi256(c.as_rgb().unwrap());
            let chrome = [idx(&t.background), idx(&t.selection), idx(&t.border)];
            assert!(
                chrome[0] != chrome[1] && chrome[0] != chrome[2],
                "{name}: chrome collapses to {chrome:?}"
            );
            let status = [
                idx(&t.status_working),
                idx(&t.status_waiting),
                idx(&t.success),
                idx(&t.danger),
                idx(&t.status_idle),
            ];
            for (i, a) in status.iter().enumerate() {
                for b in &status[i + 1..] {
                    assert_ne!(a, b, "{name}: status colours collapse to {status:?}");
                }
            }
        }
    }

    #[test]
    fn backend_downgrades_only_rgb_and_only_in_256_mode() {
        let mut cell = Cell::default();
        cell.fg = Color::Rgb(0x72, 0xa7, 0xf5);
        cell.bg = Color::Red;

        let mut backend = ColorBackend::new(TestBackend::new(1, 1), ColorDepth::Ansi256);
        backend.draw(std::iter::once((0, 0, &cell))).unwrap();
        let drawn = &backend.inner.buffer()[(0, 0)];
        assert_eq!(drawn.fg, Color::Indexed(rgb_to_ansi256((0x72, 0xa7, 0xf5))));
        assert_eq!(drawn.bg, Color::Red);

        let mut backend = ColorBackend::new(TestBackend::new(1, 1), ColorDepth::TrueColor);
        backend.draw(std::iter::once((0, 0, &cell))).unwrap();
        assert_eq!(backend.inner.buffer()[(0, 0)].fg, cell.fg);
    }

    #[test]
    fn color_mode_parsing_is_lenient() {
        let parse = |json: &str| serde_json::from_str::<ColorMode>(json).unwrap();
        assert_eq!(parse("\"auto\""), ColorMode::Auto);
        assert_eq!(parse("\"truecolor\""), ColorMode::TrueColor);
        assert_eq!(parse("\"24bit\""), ColorMode::TrueColor);
        assert_eq!(parse("\"256\""), ColorMode::Ansi256);
        assert_eq!(parse("256"), ColorMode::Ansi256);
        assert_eq!(parse("\"bogus\""), ColorMode::Auto);
        assert_eq!(
            serde_json::to_string(&ColorMode::Ansi256).unwrap(),
            "\"256\""
        );
    }

    #[test]
    fn auto_mode_follows_the_environment() {
        let env = TermEnv::from_pairs(&[("COLORTERM", "truecolor")]);
        assert_eq!(
            resolve_depth(ColorMode::Auto, &env).0,
            ColorDepth::TrueColor
        );
        assert_eq!(
            resolve_depth(ColorMode::Ansi256, &env).0,
            ColorDepth::Ansi256
        );
        let env = TermEnv::from_pairs(&[("TERM", "xterm-256color")]);
        assert_eq!(resolve_depth(ColorMode::Auto, &env).0, ColorDepth::Ansi256);
        assert_eq!(
            resolve_depth(ColorMode::TrueColor, &env).0,
            ColorDepth::TrueColor
        );
    }
}
