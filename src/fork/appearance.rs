//! AIW appearance settings (stored at the top level of `config.json`, via a
//! flattened field on `AppConfig`) and the startup steps that use them.
//!
//! ```json
//! "color_mode": "auto",              // "auto" | "truecolor" | "256"
//! "theme_follow_terminal": false,    // pick theme_dark / theme_light from the terminal
//! "theme_dark": "ember",
//! "theme_light": "paper",
//! "nerd_font_confirmed": true        // set by the first-run icon check
//! ```

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};

use super::color::{ColorDepth, ColorMode, resolve_depth};
use super::termbg::{self, Background};
use super::terminal::TermEnv;
use crate::theme::{Theme, ThemeName};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub color_mode: ColorMode,
    pub theme_follow_terminal: bool,
    #[serde(deserialize_with = "lenient_dark_theme")]
    pub theme_dark: ThemeName,
    #[serde(deserialize_with = "lenient_light_theme")]
    pub theme_light: ThemeName,
    /// `Some(true)`: the user answered the icon check. `Some(false)`: the
    /// check was postponed and runs again next start. `None`: never asked
    /// (configs written before AIW; those keep their `nerd_font` as is).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nerd_font_confirmed: Option<bool>,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            color_mode: ColorMode::Auto,
            theme_follow_terminal: false,
            theme_dark: ThemeName::Ember,
            theme_light: ThemeName::Paper,
            nerd_font_confirmed: None,
        }
    }
}

/// An unknown theme name must not make the whole config unreadable (the
/// loader falls back to defaults on any parse error), so these keys fall
/// back to their own defaults instead.
fn lenient_theme<'de, D: Deserializer<'de>>(
    deserializer: D,
    fallback: ThemeName,
) -> Result<ThemeName, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_str()
        .and_then(|name| name.parse().ok())
        .unwrap_or(fallback))
}

fn lenient_dark_theme<'de, D: Deserializer<'de>>(d: D) -> Result<ThemeName, D::Error> {
    lenient_theme(d, ThemeName::Ember)
}

fn lenient_light_theme<'de, D: Deserializer<'de>>(d: D) -> Result<ThemeName, D::Error> {
    lenient_theme(d, ThemeName::Paper)
}

/// Whether the first-run icon check should be shown. Must be called before
/// `load_config`, which writes a default `config.json` on first launch.
///
/// - no config file yet (first launch) → ask;
/// - `nerd_font_confirmed: false` (postponed last time) → ask;
/// - an existing config that never stored `nerd_font` → ask;
/// - anything else (answered, or an AIW config that already has
///   `nerd_font`) → leave it alone.
pub fn icon_check_needed(config_path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(config_path) else {
        return !config_path.exists();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    match json.get("nerd_font_confirmed").and_then(|v| v.as_bool()) {
        Some(confirmed) => !confirmed,
        None => json.get("nerd_font").is_none(),
    }
}

/// What startup decided before the TUI took over the terminal.
#[derive(Debug, Clone)]
pub struct Startup {
    pub color_depth: ColorDepth,
    pub color_reason: String,
    /// The theme to show when following the terminal, if that is on and the
    /// background could be detected.
    pub follow_theme: Option<ThemeName>,
    pub background: Option<(Background, String)>,
    pub icon_check: bool,
}

impl Startup {
    /// Runs before raw mode: may query the terminal (OSC 11) when
    /// `theme_follow_terminal` is on and `COLORFGBG` does not answer.
    pub fn detect(appearance: &AppearanceConfig, icon_check: bool) -> Self {
        let env = TermEnv::current();
        let (color_depth, color_reason) = resolve_depth(appearance.color_mode, &env);
        let background = if appearance.theme_follow_terminal {
            termbg::detect(&env, true)
        } else {
            None
        };
        let follow_theme = background
            .as_ref()
            .map(|(bg, _)| theme_for_background(appearance, *bg));
        Self {
            color_depth,
            color_reason,
            follow_theme,
            background,
            icon_check,
        }
    }

    /// Applies the followed theme (in memory; `theme` in the file is the
    /// fallback for when detection fails).
    pub fn apply(&self, app: &mut crate::app::App) {
        let background = match &self.background {
            Some((bg, why)) => format!("{} ({why})", bg.label()),
            None if app.config.appearance.theme_follow_terminal => "not detected".to_string(),
            None => "not checked".to_string(),
        };
        app.log_info(
            "appearance",
            format!(
                "colour: {} ({}); terminal background: {background}",
                self.color_depth.label(),
                self.color_reason
            ),
        );
        if let Some(name) = self.follow_theme {
            app.config.theme = name;
            let mut theme = Theme::load(&name);
            theme.set_transparent(app.config.transparent_background);
            app.theme = theme;
        }
    }
}

pub fn theme_for_background(appearance: &AppearanceConfig, background: Background) -> ThemeName {
    match background {
        Background::Dark => appearance.theme_dark,
        Background::Light => appearance.theme_light,
    }
}

/// When following the terminal, picking a theme in `T` also makes it the
/// theme for that background, so the choice sticks next time.
pub fn remember_picked_theme(appearance: &mut AppearanceConfig, name: ThemeName) {
    if !appearance.theme_follow_terminal {
        return;
    }
    if Theme::load(&name).is_light() {
        appearance.theme_light = name;
    } else {
        appearance.theme_dark = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_round_trip() {
        let config = AppearanceConfig::default();
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["color_mode"], "auto");
        assert_eq!(json["theme_dark"], "ember");
        assert_eq!(json["theme_light"], "paper");
        assert_eq!(json["theme_follow_terminal"], false);
        assert!(json.get("nerd_font_confirmed").is_none());
    }

    #[test]
    fn unknown_values_never_break_the_config() {
        let config: AppearanceConfig = serde_json::from_str(
            r#"{"color_mode":"sixteen","theme_dark":"nope","theme_light":42}"#,
        )
        .unwrap();
        assert_eq!(config.color_mode, ColorMode::Auto);
        assert_eq!(config.theme_dark, ThemeName::Ember);
        assert_eq!(config.theme_light, ThemeName::Paper);
    }

    #[test]
    fn app_config_carries_the_appearance_keys_at_top_level() {
        let config: crate::app::AppConfig = serde_json::from_str(
            r#"{"nerd_font":false,"theme":"nord","color_mode":"256","theme_follow_terminal":true,"theme_light":"porcelain"}"#,
        )
        .unwrap();
        assert_eq!(config.theme, ThemeName::Nord, "a saved theme still loads");
        assert_eq!(config.appearance.color_mode, ColorMode::Ansi256);
        assert!(config.appearance.theme_follow_terminal);
        assert_eq!(config.appearance.theme_light, ThemeName::Porcelain);
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["color_mode"], "256");
        assert_eq!(json["theme_light"], "porcelain");
        assert!(json.get("appearance").is_none());
    }

    #[test]
    fn fresh_configs_default_to_ember() {
        assert_eq!(crate::app::AppConfig::default().theme, ThemeName::Ember);
        let config: crate::app::AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.theme, ThemeName::Ember);
    }

    #[test]
    fn icon_check_rules() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert!(icon_check_needed(&path), "first launch");
        std::fs::write(&path, r#"{"nerd_font":true}"#).unwrap();
        assert!(!icon_check_needed(&path), "existing AIW config");
        std::fs::write(&path, r#"{"theme":"nord"}"#).unwrap();
        assert!(icon_check_needed(&path), "nerd_font never stored");
        std::fs::write(&path, r#"{"nerd_font":true,"nerd_font_confirmed":false}"#).unwrap();
        assert!(icon_check_needed(&path), "postponed");
        std::fs::write(&path, r#"{"nerd_font":false,"nerd_font_confirmed":true}"#).unwrap();
        assert!(!icon_check_needed(&path), "answered");
    }

    #[test]
    fn following_picks_the_matching_slot() {
        let mut appearance = AppearanceConfig {
            theme_follow_terminal: true,
            ..AppearanceConfig::default()
        };
        assert_eq!(
            theme_for_background(&appearance, Background::Light),
            ThemeName::Paper
        );
        remember_picked_theme(&mut appearance, ThemeName::Porcelain);
        remember_picked_theme(&mut appearance, ThemeName::Nord);
        assert_eq!(appearance.theme_light, ThemeName::Porcelain);
        assert_eq!(appearance.theme_dark, ThemeName::Nord);

        let mut off = AppearanceConfig::default();
        remember_picked_theme(&mut off, ThemeName::Porcelain);
        assert_eq!(off.theme_light, ThemeName::Paper);
    }
}
