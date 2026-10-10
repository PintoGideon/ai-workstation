//! `aiw fonts`: what AIW knows about the terminal's colours and fonts, and a
//! ready-to-paste font config for the common terminals.
//!
//! Fonts belong to the terminal, not to AIW, so this only reports and
//! explains. It never downloads anything and makes no network calls; the only
//! things it reads are environment variables, AIW's own config (read-only)
//! and the names of files in the usual font directories.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::color::resolve_depth;
use super::terminal::{TermEnv, TerminalKind, detect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontChoice {
    pub key: &'static str,
    pub family: &'static str,
    /// The Nerd Font build's family name (for terminals without fallback).
    pub nerd_family: &'static str,
    pub brew_cask: &'static str,
}

pub const FONTS: [FontChoice; 4] = [
    FontChoice {
        key: "jetbrains",
        family: "JetBrains Mono",
        nerd_family: "JetBrainsMono Nerd Font Mono",
        brew_cask: "font-jetbrains-mono",
    },
    FontChoice {
        key: "geist",
        family: "Geist Mono",
        nerd_family: "GeistMono Nerd Font Mono",
        brew_cask: "font-geist-mono",
    },
    FontChoice {
        key: "iosevka",
        family: "Iosevka",
        nerd_family: "Iosevka Nerd Font Mono",
        brew_cask: "font-iosevka",
    },
    FontChoice {
        key: "fira",
        family: "Fira Code",
        nerd_family: "FiraCode Nerd Font Mono",
        brew_cask: "font-fira-code",
    },
];

const SYMBOLS_FONT: &str = "Symbols Nerd Font Mono";

pub fn font_choice(key: Option<&str>) -> Result<FontChoice, String> {
    let Some(key) = key else {
        return Ok(FONTS[0]);
    };
    let wanted = key.to_ascii_lowercase().replace([' ', '-', '_'], "");
    FONTS
        .iter()
        .copied()
        .find(|f| {
            wanted.starts_with(f.key) || f.family.to_ascii_lowercase().replace(' ', "") == wanted
        })
        .ok_or_else(|| {
            let keys: Vec<&str> = FONTS.iter().map(|f| f.key).collect();
            format!("unknown font {key:?}; choose one of: {}", keys.join(", "))
        })
}

/// The five terminals we have snippets for.
pub const SNIPPET_TERMINALS: [TerminalKind; 5] = [
    TerminalKind::Ghostty,
    TerminalKind::ITerm2,
    TerminalKind::WezTerm,
    TerminalKind::Kitty,
    TerminalKind::Alacritty,
];

pub fn terminal_by_name(name: &str) -> Option<TerminalKind> {
    match name.to_ascii_lowercase().as_str() {
        "ghostty" => Some(TerminalKind::Ghostty),
        "iterm" | "iterm2" => Some(TerminalKind::ITerm2),
        "wezterm" => Some(TerminalKind::WezTerm),
        "kitty" => Some(TerminalKind::Kitty),
        "alacritty" => Some(TerminalKind::Alacritty),
        _ => None,
    }
}

/// Ready-to-paste config for one terminal, or `None` if we have none for it.
pub fn snippet(kind: TerminalKind, font: FontChoice) -> Option<String> {
    let family = font.family;
    let nerd = font.nerd_family;
    let text = match kind {
        TerminalKind::Ghostty => format!(
            "# ~/.config/ghostty/config  (reload: cmd/ctrl+shift+,)\n\
             font-family = \"{family}\"\n\
             font-family = \"{SYMBOLS_FONT}\"\n\
             font-size = 14\n\
             # Ghostty also ships Nerd Font symbols built in, so icons work\n\
             # even without the second font-family line."
        ),
        TerminalKind::ITerm2 => format!(
            "# iTerm2 has no config file for this; in Settings > Profiles > Text:\n\
             #   Font:                                {family}, 14\n\
             #   [x] Use a different font for non-ASCII text\n\
             #   Non-ASCII font:                      {SYMBOLS_FONT}, 14\n\
             # 24-bit colour is on by default; AIW detects iTerm2 automatically."
        ),
        TerminalKind::WezTerm => format!(
            "-- ~/.wezterm.lua\n\
             local wezterm = require 'wezterm'\n\
             local config = wezterm.config_builder()\n\
             config.font = wezterm.font_with_fallback {{ '{family}', '{SYMBOLS_FONT}' }}\n\
             config.font_size = 14.0\n\
             return config\n\
             -- WezTerm bundles Nerd Font symbols too, so the fallback is optional."
        ),
        TerminalKind::Kitty => format!(
            "# ~/.config/kitty/kitty.conf  (reload: ctrl+shift+F5)\n\
             font_family      {family}\n\
             font_size        14.0\n\
             symbol_map U+E000-U+F8FF,U+F0000-U+FFFFD {SYMBOLS_FONT}"
        ),
        TerminalKind::Alacritty => format!(
            "# ~/.config/alacritty/alacritty.toml\n\
             [font]\n\
             size = 14.0\n\
             normal = {{ family = \"{nerd}\" }}\n\
             # Alacritty has no per-font fallback list, so use the Nerd Font\n\
             # build of your font (it already contains the icons)."
        ),
        _ => return None,
    };
    Some(text)
}

/// Families of Nerd Fonts installed in the usual font directories, from file
/// names only (e.g. `JetBrainsMonoNerdFontMono-Regular.ttf` →
/// `JetBrainsMono Nerd Font`).
pub fn installed_nerd_fonts(dirs: &[PathBuf]) -> Vec<String> {
    let mut found = Vec::new();
    for dir in dirs {
        scan_fonts(dir, 0, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn scan_fonts(dir: &Path, depth: usize, found: &mut Vec<String>) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_fonts(&path, depth + 1, found);
        } else if let Some(family) = nerd_family_from_file(&path) {
            found.push(family);
        }
    }
}

pub fn nerd_family_from_file(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "ttf" | "otf" | "ttc") {
        return None;
    }
    let stem: String = path.file_stem()?.to_str()?.replace(' ', "");
    let at = stem.to_ascii_lowercase().find("nerdfont")?;
    let prefix = stem[..at].trim_end_matches(['-', '_']);
    if prefix.is_empty() {
        return None;
    }
    Some(format!("{prefix} Nerd Font"))
}

fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
        dirs.push(home.join("Library/Fonts"));
    }
    for system in [
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        "/Library/Fonts",
    ] {
        dirs.push(PathBuf::from(system));
    }
    dirs
}

pub struct Report<'a> {
    pub env: &'a TermEnv,
    pub config: &'a crate::app::AppConfig,
    pub nerd_fonts: &'a [String],
    pub font: FontChoice,
    /// Which terminals to print snippets for.
    pub snippets_for: Vec<TerminalKind>,
}

pub fn render(report: &Report) -> String {
    let env = report.env;
    let config = report.config;
    let detected = detect(env);
    let (depth, depth_reason) = resolve_depth(config.appearance.color_mode, env);
    let mut out = String::new();

    let _ = writeln!(out, "AI Workstation: fonts and colours\n");
    let _ = writeln!(
        out,
        "  Terminal     {} ({})",
        detected.kind.label(),
        detected.evidence
    );
    if env.inside_tmux() {
        let _ = writeln!(out, "               inside tmux");
    }
    let _ = writeln!(out, "  Colour       {} - {}", depth.label(), depth_reason);
    let _ = writeln!(
        out,
        "               color_mode = \"{}\"  (auto | truecolor | 256)",
        config.appearance.color_mode.as_str()
    );
    let icons = match config.appearance.nerd_font_confirmed {
        Some(true) => "set by the icon check",
        Some(false) => "icon check postponed; asks again next start",
        None => "not checked yet",
    };
    let _ = writeln!(
        out,
        "  Icons        nerd_font = {}  ({icons})",
        config.nerd_font
    );
    if report.nerd_fonts.is_empty() {
        let _ = writeln!(out, "  Nerd Fonts   none found in the usual font folders");
    } else {
        let shown: Vec<&str> = report
            .nerd_fonts
            .iter()
            .take(6)
            .map(String::as_str)
            .collect();
        let more = report.nerd_fonts.len().saturating_sub(shown.len());
        let _ = writeln!(
            out,
            "  Nerd Fonts   installed: {}{}",
            shown.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        );
    }
    let follow = if config.appearance.theme_follow_terminal {
        format!(
            "on (dark: {}, light: {})",
            config.appearance.theme_dark, config.appearance.theme_light
        )
    } else {
        "off".to_string()
    };
    let _ = writeln!(
        out,
        "  Theme        {}  (follow terminal: {follow})",
        config.theme
    );

    let _ = writeln!(
        out,
        "\nRecommended fonts: JetBrains Mono, Geist Mono, Iosevka or Fira Code,\n\
         with \"{SYMBOLS_FONT}\" as the fallback for icons. Showing {}\n\
         (pick another with --font jetbrains|geist|iosevka|fira).",
        report.font.family
    );

    for kind in &report.snippets_for {
        if let Some(text) = snippet(*kind, report.font) {
            let _ = writeln!(out, "\n== {} ==\n{text}", kind.label());
        }
    }

    let _ = writeln!(
        out,
        "\nInstalling fonts (AIW never downloads anything itself):\n\
         \x20 macOS:  brew install --cask {} font-symbols-only-nerd-font\n\
         \x20 Linux:  get \"{}\" or \"Symbols Only\" from https://www.nerdfonts.com/font-downloads,\n\
         \x20         unzip into ~/.local/share/fonts and run `fc-cache -f`.\n\
         After changing fonts, restart AIW; if icons still show as boxes set\n\
         \"nerd_font\": false in config.json (or answer n to the icon check).",
        report.font.brew_cask, report.font.nerd_family
    );
    if depth == super::color::ColorDepth::Ansi256 {
        let _ = writeln!(
            out,
            "\nColours: AIW is mapping themes to 256 colours. If your terminal does\n\
             support 24-bit colour, set COLORTERM=truecolor or \"color_mode\": \"truecolor\"."
        );
    }
    out
}

/// Entry point for `aiw fonts`.
pub fn run(all: bool, font: Option<&str>, terminal: Option<&str>) -> anyhow::Result<()> {
    let font = font_choice(font).map_err(anyhow::Error::msg)?;
    let env = TermEnv::current();
    // Read-only: `load_config` would create or migrate config.json.
    let config = crate::app::setup::read_config();
    let snippets_for = if let Some(name) = terminal {
        vec![terminal_by_name(name).ok_or_else(|| {
            anyhow::anyhow!(
                "no snippet for {name:?}; use ghostty, iterm2, wezterm, kitty or alacritty"
            )
        })?]
    } else {
        let detected = detect(&env).kind;
        if !all && SNIPPET_TERMINALS.contains(&detected) {
            vec![detected]
        } else {
            SNIPPET_TERMINALS.to_vec()
        }
    };
    let nerd_fonts = installed_nerd_fonts(&font_dirs());
    print!(
        "{}",
        render(&Report {
            env: &env,
            config: &config,
            nerd_fonts: &nerd_fonts,
            font,
            snippets_for,
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_terminal_has_a_snippet_for_every_font() {
        for kind in SNIPPET_TERMINALS {
            for font in FONTS {
                let text = snippet(kind, font).unwrap();
                assert!(
                    text.contains(font.family) || text.contains(font.nerd_family),
                    "{kind:?} / {}",
                    font.key
                );
            }
        }
        assert!(snippet(TerminalKind::Unknown, FONTS[0]).is_none());
    }

    #[test]
    fn font_choice_accepts_short_and_full_names() {
        assert_eq!(font_choice(None).unwrap().family, "JetBrains Mono");
        assert_eq!(font_choice(Some("geist")).unwrap().family, "Geist Mono");
        assert_eq!(font_choice(Some("Fira Code")).unwrap().family, "Fira Code");
        assert_eq!(
            font_choice(Some("jetbrains-mono")).unwrap().key,
            "jetbrains"
        );
        assert!(font_choice(Some("comic sans")).is_err());
    }

    #[test]
    fn nerd_font_families_from_file_names() {
        let family = |name: &str| nerd_family_from_file(Path::new(name));
        assert_eq!(
            family("JetBrainsMonoNerdFontMono-Regular.ttf").as_deref(),
            Some("JetBrainsMono Nerd Font")
        );
        assert_eq!(
            family("SymbolsNerdFont-Regular.ttf").as_deref(),
            Some("Symbols Nerd Font")
        );
        assert_eq!(
            family("Fira Code Nerd Font Complete.otf").as_deref(),
            Some("FiraCode Nerd Font")
        );
        assert_eq!(family("JetBrainsMono-Regular.ttf"), None);
        assert_eq!(family("NerdFont.txt"), None);
    }

    #[test]
    fn report_shows_terminal_colour_and_settings() {
        let env = TermEnv::from_pairs(&[("TERM", "xterm-256color"), ("KITTY_WINDOW_ID", "1")]);
        let config = crate::app::AppConfig::default();
        let text = render(&Report {
            env: &env,
            config: &config,
            nerd_fonts: &["JetBrainsMono Nerd Font".to_string()],
            font: FONTS[0],
            snippets_for: vec![TerminalKind::Kitty],
        });
        assert!(text.contains("Kitty (KITTY_WINDOW_ID is set)"), "{text}");
        assert!(text.contains("24-bit"), "{text}");
        assert!(text.contains("color_mode = \"auto\""), "{text}");
        assert!(text.contains("nerd_font = true"), "{text}");
        assert!(text.contains("symbol_map"), "{text}");
        assert!(!text.contains("== Ghostty =="), "{text}");
    }
}
