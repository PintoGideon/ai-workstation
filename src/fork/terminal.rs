//! Which terminal emulator AIW is running in, and whether it draws 24-bit
//! colour. Pure functions over an environment snapshot so they are testable;
//! [`TermEnv::current`] reads the real environment.

use std::collections::HashMap;

/// The environment variables terminal detection looks at.
const KEYS: &[&str] = &[
    "TERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "COLORTERM",
    "COLORFGBG",
    "TMUX",
    "GHOSTTY_RESOURCES_DIR",
    "ITERM_SESSION_ID",
    "WEZTERM_EXECUTABLE",
    "WEZTERM_PANE",
    "KITTY_WINDOW_ID",
    "ALACRITTY_SOCKET",
    "ALACRITTY_LOG",
    "ALACRITTY_WINDOW_ID",
    "VTE_VERSION",
    "KONSOLE_VERSION",
    "WT_SESSION",
    "VSCODE_INJECTION",
];

#[derive(Debug, Clone, Default)]
pub struct TermEnv {
    vars: HashMap<String, String>,
}

impl TermEnv {
    pub fn current() -> Self {
        let vars = KEYS
            .iter()
            .filter_map(|key| {
                std::env::var(key)
                    .ok()
                    .map(|value| (key.to_string(), value))
            })
            .collect();
        Self { vars }
    }

    #[cfg(test)]
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self {
            vars: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn inside_tmux(&self) -> bool {
        self.has("TMUX") || self.get("TERM_PROGRAM") == Some("tmux")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    Ghostty,
    ITerm2,
    WezTerm,
    Kitty,
    Alacritty,
    VsCode,
    AppleTerminal,
    WindowsTerminal,
    Konsole,
    Vte,
    Unknown,
}

impl TerminalKind {
    pub fn label(self) -> &'static str {
        match self {
            TerminalKind::Ghostty => "Ghostty",
            TerminalKind::ITerm2 => "iTerm2",
            TerminalKind::WezTerm => "WezTerm",
            TerminalKind::Kitty => "Kitty",
            TerminalKind::Alacritty => "Alacritty",
            TerminalKind::VsCode => "VS Code terminal",
            TerminalKind::AppleTerminal => "Apple Terminal",
            TerminalKind::WindowsTerminal => "Windows Terminal",
            TerminalKind::Konsole => "Konsole",
            TerminalKind::Vte => "GNOME Terminal / VTE-based",
            TerminalKind::Unknown => "unknown",
        }
    }
}

/// The detected terminal and the variable that gave it away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub kind: TerminalKind,
    pub evidence: String,
}

/// Identify the terminal emulator. Terminal-specific variables win over
/// `TERM_PROGRAM` because inside tmux `TERM_PROGRAM` says `tmux` while the
/// emulator's own variables are usually inherited from where tmux started.
pub fn detect(env: &TermEnv) -> Detected {
    let found = |kind, evidence: String| Detected { kind, evidence };
    let by_var: &[(&str, TerminalKind)] = &[
        ("GHOSTTY_RESOURCES_DIR", TerminalKind::Ghostty),
        ("ITERM_SESSION_ID", TerminalKind::ITerm2),
        ("WEZTERM_EXECUTABLE", TerminalKind::WezTerm),
        ("WEZTERM_PANE", TerminalKind::WezTerm),
        ("KITTY_WINDOW_ID", TerminalKind::Kitty),
        ("ALACRITTY_SOCKET", TerminalKind::Alacritty),
        ("ALACRITTY_LOG", TerminalKind::Alacritty),
        ("ALACRITTY_WINDOW_ID", TerminalKind::Alacritty),
        ("WT_SESSION", TerminalKind::WindowsTerminal),
        ("KONSOLE_VERSION", TerminalKind::Konsole),
    ];
    for (key, kind) in by_var {
        if env.has(key) {
            return found(*kind, format!("{key} is set"));
        }
    }
    if let Some(program) = env.get("TERM_PROGRAM") {
        let kind = match program {
            "ghostty" => Some(TerminalKind::Ghostty),
            "iTerm.app" => Some(TerminalKind::ITerm2),
            "WezTerm" => Some(TerminalKind::WezTerm),
            "vscode" => Some(TerminalKind::VsCode),
            "Apple_Terminal" => Some(TerminalKind::AppleTerminal),
            _ => None,
        };
        if let Some(kind) = kind {
            return found(kind, format!("TERM_PROGRAM={program}"));
        }
    }
    if let Some(term) = env.get("TERM") {
        let kind = match term {
            "xterm-ghostty" => Some(TerminalKind::Ghostty),
            "xterm-kitty" => Some(TerminalKind::Kitty),
            "wezterm" => Some(TerminalKind::WezTerm),
            "alacritty" | "alacritty-direct" => Some(TerminalKind::Alacritty),
            _ => None,
        };
        if let Some(kind) = kind {
            return found(kind, format!("TERM={term}"));
        }
    }
    if let Some(vte) = env.get("VTE_VERSION") {
        return found(TerminalKind::Vte, format!("VTE_VERSION={vte}"));
    }
    found(
        TerminalKind::Unknown,
        "no known terminal variables".to_string(),
    )
}

/// Whether the terminal draws 24-bit colour, and why we think so.
pub fn truecolor_support(env: &TermEnv) -> (bool, String) {
    if let Some(colorterm) = env.get("COLORTERM") {
        let lower = colorterm.to_ascii_lowercase();
        if lower == "truecolor" || lower == "24bit" {
            return (true, format!("COLORTERM={colorterm}"));
        }
    }
    if let Some(term) = env.get("TERM")
        && (term.contains("truecolor") || term.contains("24bit") || term.ends_with("-direct"))
    {
        return (true, format!("TERM={term}"));
    }
    // tmux keeps colours as RGB internally and downsamples to 256 colours
    // itself when the outer terminal lacks RGB, so 24-bit output is safe.
    if env.inside_tmux() {
        return (
            true,
            "inside tmux (tmux converts colours itself)".to_string(),
        );
    }
    let detected = detect(env);
    match detected.kind {
        TerminalKind::Ghostty
        | TerminalKind::ITerm2
        | TerminalKind::WezTerm
        | TerminalKind::Kitty
        | TerminalKind::Alacritty
        | TerminalKind::VsCode
        | TerminalKind::WindowsTerminal
        | TerminalKind::Konsole => (
            true,
            format!("{} ({})", detected.kind.label(), detected.evidence),
        ),
        // VTE gained 24-bit colour in 0.36 (VTE_VERSION 3600).
        TerminalKind::Vte => {
            let version: u32 = env
                .get("VTE_VERSION")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            (version >= 3600, detected.evidence)
        }
        TerminalKind::AppleTerminal => (
            false,
            "Apple Terminal (24-bit colour only on recent macOS; set color_mode to override)"
                .to_string(),
        ),
        TerminalKind::Unknown => (false, "COLORTERM is not truecolor/24bit".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_terminals_from_their_own_variables() {
        let cases: &[(&[(&str, &str)], TerminalKind)] = &[
            (&[("GHOSTTY_RESOURCES_DIR", "/x")], TerminalKind::Ghostty),
            (&[("TERM_PROGRAM", "iTerm.app")], TerminalKind::ITerm2),
            (&[("ITERM_SESSION_ID", "w0t0p0")], TerminalKind::ITerm2),
            (&[("WEZTERM_EXECUTABLE", "/w")], TerminalKind::WezTerm),
            (&[("KITTY_WINDOW_ID", "1")], TerminalKind::Kitty),
            (&[("ALACRITTY_LOG", "/tmp/a")], TerminalKind::Alacritty),
            (&[("VTE_VERSION", "7600")], TerminalKind::Vte),
            (&[("TERM", "xterm-256color")], TerminalKind::Unknown),
            // Inside tmux the emulator's own variable still wins.
            (
                &[("TERM_PROGRAM", "tmux"), ("KITTY_WINDOW_ID", "3")],
                TerminalKind::Kitty,
            ),
        ];
        for (pairs, kind) in cases {
            assert_eq!(detect(&TermEnv::from_pairs(pairs)).kind, *kind, "{pairs:?}");
        }
    }

    #[test]
    fn truecolor_detection() {
        let yes: &[&[(&str, &str)]] = &[
            &[("COLORTERM", "truecolor")],
            &[("COLORTERM", "24bit")],
            &[("TERM", "xterm-direct")],
            &[("TERM_PROGRAM", "WezTerm")],
            &[("VTE_VERSION", "6003")],
            &[("TMUX", "/tmp/tmux-1/default,1,0")],
        ];
        for pairs in yes {
            assert!(
                truecolor_support(&TermEnv::from_pairs(pairs)).0,
                "{pairs:?}"
            );
        }
        let no: &[&[(&str, &str)]] = &[
            &[("TERM", "xterm-256color")],
            &[("TERM_PROGRAM", "Apple_Terminal")],
            &[("VTE_VERSION", "3405")],
            &[],
        ];
        for pairs in no {
            assert!(
                !truecolor_support(&TermEnv::from_pairs(pairs)).0,
                "{pairs:?}"
            );
        }
    }
}
