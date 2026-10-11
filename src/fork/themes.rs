//! AIW's own built-in palettes: Ember (the default), Graphite, Midnight,
//! Paper and Porcelain.
//!
//! The hex values come from the AI Harness design palettes
//! (`ai_harness_ui/src/theme/builtin.ts`). Mapping onto AIW's theme roles:
//!
//! - chrome stays neutral: `border` is a quiet line, `primary` /
//!   `border_focus` / `shortcut_background` share the palette's single accent
//!   (its focus ring), titles use the foreground;
//! - status colours are the only saturated ones and mean the same in every
//!   palette: working = blue (`status_active`, `status_working`), needs you =
//!   amber (`status_waiting`, `warning`), done = green (`success`), error = red
//!   (`danger`), idle / stopped = grey (`status_idle`, `status_stopped`).
//!
//! A handful of values were nudged from the source palette so every theme
//! passes the contrast tests in `crate::fork::contrast` (idle greys that sat
//! just under 3:1, light-theme ambers that were under 3.5:1, and borders,
//! which the TUI needs a little stronger than a web UI's hairlines). Those are
//! marked "adjusted".

use crate::theme::{ColorDef, Theme};

/// `#rrggbb` → [`ColorDef`]. Only ever called with the literals below; the
/// palette tests load every theme, so a typo fails the test suite.
fn hex(value: &str) -> ColorDef {
    let value = value.trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(&value[i..i + 2], 16).expect("palette hex");
    ColorDef::rgb(channel(0), channel(2), channel(4))
}

pub(crate) fn ember() -> Theme {
    Theme {
        name: "ember".to_string(),
        background: hex("#141110"),
        text: hex("#ebe7e3"),
        text_muted: hex("#9d958d"),
        primary: hex("#d08a6c"),
        secondary: hex("#c99ad6"),
        success: hex("#86c48f"),
        warning: hex("#e3a857"),
        danger: hex("#e8696b"),
        info: hex("#6fc2be"),
        pr_merged: hex("#c99ad6"),
        pr_closed: hex("#9d958d"),
        border: hex("#463e38"), // adjusted
        border_focus: hex("#d08a6c"),
        selection: hex("#3d342e"),
        header_background: hex("#1c1917"),
        shortcut_background: hex("#d08a6c"),
        shortcut_text: hex("#141110"),
        scrollbar: hex("#463e38"),
        project_title: hex("#f4f0ec"),
        feature_title: hex("#ebe7e3"),
        session_icon_claude: hex("#e39b7b"),
        session_icon_codex: hex("#d9d2ca"),
        session_icon_gemini: hex("#ddb4e6"),
        session_icon_cursor: hex("#8fd8d2"),
        session_icon_terminal: hex("#9d958d"),
        session_icon_nvim: hex("#a6c98b"),
        session_icon_vscode: hex("#7aa2e3"),
        session_icon_custom: hex("#e5b567"),
        status_active: hex("#72a7f5"),
        status_working: hex("#72a7f5"),
        status_idle: hex("#6c645d"),
        status_stopped: hex("#6c645d"),
        status_waiting: hex("#f0a640"),
        status_detail: hex("#6fc2be"),
        mode_vibeless: hex("#86c48f"),
        mode_vibe: hex("#e3a857"),
        mode_supervibe: hex("#c99ad6"),
        usage_low: hex("#86c48f"),
        usage_medium: hex("#e3a857"),
        usage_high: hex("#e8696b"),
        transparent: false,
    }
}

pub(crate) fn graphite() -> Theme {
    Theme {
        name: "graphite".to_string(),
        background: hex("#111214"),
        text: hex("#e4e5e7"),
        text_muted: hex("#8f9198"),
        primary: hex("#8b93ff"),
        secondary: hex("#c592ff"),
        success: hex("#6fcf8a"),
        warning: hex("#e8b44f"),
        danger: hex("#f2555a"),
        info: hex("#5fd3e3"),
        pr_merged: hex("#c592ff"),
        pr_closed: hex("#8f9198"),
        border: hex("#3b3d44"), // adjusted
        border_focus: hex("#8b93ff"),
        selection: hex("#2f3138"),
        header_background: hex("#17181b"),
        shortcut_background: hex("#8b93ff"),
        shortcut_text: hex("#111214"),
        scrollbar: hex("#3b3d44"),
        project_title: hex("#f2f3f5"),
        feature_title: hex("#e4e5e7"),
        session_icon_claude: hex("#f4b46b"),
        session_icon_codex: hex("#d4d6da"),
        session_icon_gemini: hex("#d9b4ff"),
        session_icon_cursor: hex("#8ce4ef"),
        session_icon_terminal: hex("#8f9198"),
        session_icon_nvim: hex("#9be3a6"),
        session_icon_vscode: hex("#94c0ff"),
        session_icon_custom: hex("#e8c26b"),
        status_active: hex("#6ea8fe"),
        status_working: hex("#6ea8fe"),
        status_idle: hex("#63666e"),    // adjusted from #5d6067 (2.98:1)
        status_stopped: hex("#63666e"), // adjusted
        status_waiting: hex("#f5a524"),
        status_detail: hex("#5fd3e3"),
        mode_vibeless: hex("#6fcf8a"),
        mode_vibe: hex("#e8b44f"),
        mode_supervibe: hex("#c592ff"),
        usage_low: hex("#6fcf8a"),
        usage_medium: hex("#e8b44f"),
        usage_high: hex("#f2555a"),
        transparent: false,
    }
}

pub(crate) fn midnight() -> Theme {
    Theme {
        name: "midnight".to_string(),
        background: hex("#0b0e14"),
        text: hex("#d6deeb"),
        text_muted: hex("#7f8aa3"),
        primary: hex("#7aa2f7"),
        secondary: hex("#bb9af7"),
        success: hex("#73daca"),
        warning: hex("#e0af68"),
        danger: hex("#f7768e"),
        info: hex("#7dcfff"),
        pr_merged: hex("#bb9af7"),
        pr_closed: hex("#7f8aa3"),
        border: hex("#2e3a52"), // adjusted
        border_focus: hex("#7aa2f7"),
        selection: hex("#24304a"),
        header_background: hex("#10141c"),
        shortcut_background: hex("#7aa2f7"),
        shortcut_text: hex("#0b0e14"),
        scrollbar: hex("#2e3a52"),
        project_title: hex("#e8eef8"),
        feature_title: hex("#d6deeb"),
        session_icon_claude: hex("#f0c88e"),
        session_icon_codex: hex("#c0caf5"),
        session_icon_gemini: hex("#cfb5ff"),
        session_icon_cursor: hex("#a3ddff"),
        session_icon_terminal: hex("#7f8aa3"),
        session_icon_nvim: hex("#9ece6a"),
        session_icon_vscode: hex("#9ab8ff"),
        session_icon_custom: hex("#e0af68"),
        status_active: hex("#7aa2f7"),
        status_working: hex("#7aa2f7"),
        status_idle: hex("#5c6784"),    // adjusted from #4f5872 (2.73:1)
        status_stopped: hex("#5c6784"), // adjusted
        status_waiting: hex("#ff9e64"),
        status_detail: hex("#7dcfff"),
        mode_vibeless: hex("#73daca"),
        mode_vibe: hex("#e0af68"),
        mode_supervibe: hex("#bb9af7"),
        usage_low: hex("#73daca"),
        usage_medium: hex("#e0af68"),
        usage_high: hex("#f7768e"),
        transparent: false,
    }
}

pub(crate) fn paper() -> Theme {
    Theme {
        name: "paper".to_string(),
        background: hex("#f7f2e8"),
        text: hex("#2b241d"),
        text_muted: hex("#6f6556"),
        primary: hex("#2f6f9f"),
        secondary: hex("#8a4f9a"),
        success: hex("#3f8a55"),
        warning: hex("#a3620a"), // the palette's "blocked" amber; #b77a1c is 3.2:1
        danger: hex("#c0413d"),
        info: hex("#2a7f84"),
        pr_merged: hex("#8a4f9a"),
        pr_closed: hex("#6f6556"),
        border: hex("#c9bca3"), // adjusted
        border_focus: hex("#2f6f9f"),
        selection: hex("#e7dcc5"),
        header_background: hex("#f0e9db"),
        shortcut_background: hex("#2f6f9f"),
        shortcut_text: hex("#f7f2e8"),
        scrollbar: hex("#c9bca3"),
        project_title: hex("#1f1913"),
        feature_title: hex("#2b241d"),
        session_icon_claude: hex("#a1452e"),
        session_icon_codex: hex("#4a4034"),
        session_icon_gemini: hex("#a463b5"),
        session_icon_cursor: hex("#2b8388"),
        session_icon_terminal: hex("#6f6556"),
        session_icon_nvim: hex("#3f7d45"),
        session_icon_vscode: hex("#2f62a8"),
        session_icon_custom: hex("#9a6a12"),
        status_active: hex("#2f6fc4"),
        status_working: hex("#2f6fc4"),
        status_idle: hex("#8a7f6e"),    // adjusted from #a39886 (2.55:1)
        status_stopped: hex("#8a7f6e"), // adjusted
        status_waiting: hex("#a3620a"),
        status_detail: hex("#2a7f84"),
        mode_vibeless: hex("#3f8a55"),
        mode_vibe: hex("#a3620a"),
        mode_supervibe: hex("#8a4f9a"),
        usage_low: hex("#3f8a55"),
        usage_medium: hex("#a3620a"),
        usage_high: hex("#c0413d"),
        transparent: false,
    }
}

pub(crate) fn porcelain() -> Theme {
    Theme {
        name: "porcelain".to_string(),
        background: hex("#ffffff"),
        text: hex("#18181b"),
        text_muted: hex("#6b6b74"),
        primary: hex("#4f6bed"),
        secondary: hex("#8e44ad"),
        success: hex("#2f8f4e"),
        warning: hex("#b45f06"), // the palette's "blocked" amber; #c27c0e is 3.4:1
        danger: hex("#d33b3b"),
        info: hex("#18808a"),
        pr_merged: hex("#8e44ad"),
        pr_closed: hex("#6b6b74"),
        border: hex("#c8c8cf"), // adjusted
        border_focus: hex("#4f6bed"),
        selection: hex("#dbe4fb"),
        header_background: hex("#fafafa"),
        shortcut_background: hex("#4f6bed"),
        shortcut_text: hex("#ffffff"),
        scrollbar: hex("#c8c8cf"),
        project_title: hex("#0f0f12"),
        feature_title: hex("#18181b"),
        session_icon_claude: hex("#a1349b"),
        session_icon_codex: hex("#3f3f46"),
        session_icon_gemini: hex("#a35cc2"),
        session_icon_cursor: hex("#1b8790"),
        session_icon_terminal: hex("#6b6b74"),
        session_icon_nvim: hex("#2c7c3f"),
        session_icon_vscode: hex("#2f5fd0"),
        session_icon_custom: hex("#9a6700"),
        status_active: hex("#2f6fe0"),
        status_working: hex("#2f6fe0"),
        status_idle: hex("#85858e"),    // adjusted from #a1a1aa (2.56:1)
        status_stopped: hex("#85858e"), // adjusted
        status_waiting: hex("#b45f06"),
        status_detail: hex("#18808a"),
        mode_vibeless: hex("#2f8f4e"),
        mode_vibe: hex("#b45f06"),
        mode_supervibe: hex("#8e44ad"),
        usage_low: hex("#2f8f4e"),
        usage_medium: hex("#b45f06"),
        usage_high: hex("#d33b3b"),
        transparent: false,
    }
}

#[cfg(test)]
mod tests {
    use crate::theme::{Theme, ThemeName};

    const ACC_THEMES: [ThemeName; 5] = [
        ThemeName::Ember,
        ThemeName::Graphite,
        ThemeName::Midnight,
        ThemeName::Paper,
        ThemeName::Porcelain,
    ];

    #[test]
    fn acc_themes_load_under_their_config_names() {
        for name in ACC_THEMES {
            let theme = Theme::load(&name);
            assert_eq!(theme.name, name.to_string());
            assert_eq!(name.to_string().parse::<ThemeName>().unwrap(), name);
        }
    }

    #[test]
    fn acc_themes_are_grouped_by_lightness() {
        assert!(!Theme::load(&ThemeName::Ember).is_light());
        assert!(!Theme::load(&ThemeName::Graphite).is_light());
        assert!(!Theme::load(&ThemeName::Midnight).is_light());
        assert!(Theme::load(&ThemeName::Paper).is_light());
        assert!(Theme::load(&ThemeName::Porcelain).is_light());
    }

    /// Status semantics are shared: the working spinner and the running dot
    /// agree, "needs you" is one amber, idle and stopped are one quiet grey,
    /// and no two of working / needs-you / done / error / idle collide.
    #[test]
    fn acc_status_colours_are_consistent() {
        for name in ACC_THEMES {
            let t = Theme::load(&name);
            let rgb = |c: &crate::theme::ColorDef| c.as_rgb().unwrap();
            assert_eq!(rgb(&t.status_working), rgb(&t.status_active), "{name}");
            assert_eq!(rgb(&t.status_idle), rgb(&t.status_stopped), "{name}");
            assert_eq!(rgb(&t.primary), rgb(&t.border_focus), "{name}");
            let distinct = [
                rgb(&t.status_working),
                rgb(&t.status_waiting),
                rgb(&t.success),
                rgb(&t.danger),
                rgb(&t.status_idle),
            ];
            for (i, a) in distinct.iter().enumerate() {
                for b in &distinct[i + 1..] {
                    assert_ne!(a, b, "{name}: two status roles share a colour");
                }
            }
        }
    }
}
