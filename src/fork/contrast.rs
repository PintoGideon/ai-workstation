//! WCAG 2.x contrast maths, plus the readability checks every built-in theme
//! is held to.
//!
//! AIW's own themes (Ember, Graphite, Midnight, Paper, Porcelain) must pass
//! all checks; the inherited AIW themes are only reported (see
//! `upstream_theme_report` in the tests, run with `--nocapture`). The checks
//! are test-only; the app itself only needs `relative_luminance`.

#[cfg(test)]
use crate::theme::{ColorDef, Theme};

/// WCAG relative luminance of an sRGB colour, 0.0 (black) ..= 1.0 (white).
pub fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
    fn channel(c: u8) -> f64 {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

#[cfg(test)]
/// WCAG contrast ratio, 1.0 ..= 21.0.
pub fn contrast_ratio(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
/// One readability requirement: `fg` on `bg` must reach `min`.
pub struct Check {
    pub label: &'static str,
    pub min: f64,
    pub ratio: Option<f64>,
}

#[cfg(test)]
impl Check {
    pub fn passes(&self) -> bool {
        self.ratio.is_some_and(|ratio| ratio + 1e-9 >= self.min)
    }
}

#[cfg(test)]
fn check(label: &'static str, fg: &ColorDef, bg: &ColorDef, min: f64) -> Check {
    let ratio = fg
        .as_rgb()
        .zip(bg.as_rgb())
        .map(|(fg, bg)| contrast_ratio(fg, bg));
    Check { label, min, ratio }
}

#[cfg(test)]
/// The checks from the theming spec: body text ≥ 7:1, muted text ≥ 4.5:1,
/// status colours ≥ 3:1 (all against the theme background).
pub fn core_checks(t: &Theme) -> Vec<Check> {
    let bg = &t.background;
    vec![
        check("text", &t.text, bg, 7.0),
        check("text_muted", &t.text_muted, bg, 4.5),
        check("success", &t.success, bg, 3.0),
        check("warning", &t.warning, bg, 3.0),
        check("danger", &t.danger, bg, 3.0),
        check("info", &t.info, bg, 3.0),
        check("status_active", &t.status_active, bg, 3.0),
        check("status_working", &t.status_working, bg, 3.0),
        check("status_waiting", &t.status_waiting, bg, 3.0),
        check("status_idle", &t.status_idle, bg, 3.0),
        check("status_stopped", &t.status_stopped, bg, 3.0),
    ]
}

#[cfg(test)]
/// Extra pairs AIW's own themes also guarantee, because the UI draws them:
/// text on the selected row and the header, accent-coloured text, and the
/// dark-on-accent labels (`shortcut_text` on `primary` / `info`).
pub fn extended_checks(t: &Theme) -> Vec<Check> {
    vec![
        check("text on selection", &t.text, &t.selection, 7.0),
        check("text on header", &t.text, &t.header_background, 7.0),
        check("primary", &t.primary, &t.background, 4.5),
        check(
            "shortcut_text on primary",
            &t.shortcut_text,
            &t.primary,
            4.5,
        ),
        check("shortcut_text on info", &t.shortcut_text, &t.info, 3.0),
        check("project_title", &t.project_title, &t.background, 7.0),
        check("feature_title", &t.feature_title, &t.background, 7.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeName;

    const ACC_THEMES: [ThemeName; 5] = [
        ThemeName::Ember,
        ThemeName::Graphite,
        ThemeName::Midnight,
        ThemeName::Paper,
        ThemeName::Porcelain,
    ];

    #[test]
    fn contrast_ratio_matches_wcag_reference_values() {
        assert!((contrast_ratio((0, 0, 0), (255, 255, 255)) - 21.0).abs() < 1e-9);
        assert!((contrast_ratio((255, 255, 255), (255, 255, 255)) - 1.0).abs() < 1e-9);
        // #767676 on white is the classic 4.54:1 AA boundary grey.
        let grey = contrast_ratio((0x76, 0x76, 0x76), (255, 255, 255));
        assert!((grey - 4.54).abs() < 0.01, "{grey}");
    }

    #[test]
    fn acc_themes_meet_every_contrast_requirement() {
        let mut failures = Vec::new();
        for name in ACC_THEMES {
            let theme = Theme::load(&name);
            for check in core_checks(&theme)
                .into_iter()
                .chain(extended_checks(&theme))
            {
                if !check.passes() {
                    failures.push(format!(
                        "{name}: {} {:.2} < {}",
                        check.label,
                        check.ratio.unwrap_or(0.0),
                        check.min
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn acc_theme_borders_are_quiet_but_visible() {
        for name in ACC_THEMES {
            let t = Theme::load(&name);
            let bg = t.background.as_rgb().unwrap();
            let border = contrast_ratio(t.border.as_rgb().unwrap(), bg);
            let selection = contrast_ratio(t.selection.as_rgb().unwrap(), bg);
            assert!((1.4..2.5).contains(&border), "{name}: border {border:.2}");
            assert!(selection >= 1.2, "{name}: selection {selection:.2}");
        }
    }

    /// Reports (does not enforce) how the inherited AIW themes fare against
    /// the same core checks. Run with `--nocapture` to read it.
    #[test]
    fn upstream_theme_report() {
        let mut lines = Vec::new();
        for name in Theme::list() {
            if ACC_THEMES.contains(&name) {
                continue;
            }
            let theme = Theme::load(&name);
            let failing: Vec<String> = core_checks(&theme)
                .into_iter()
                .filter(|check| !check.passes())
                .map(|check| match check.ratio {
                    Some(ratio) => format!("{} {ratio:.2}", check.label),
                    None => format!("{} n/a", check.label),
                })
                .collect();
            if !failing.is_empty() {
                lines.push(format!("{name}: {}", failing.join(", ")));
            }
        }
        eprintln!(
            "contrast report for upstream themes ({} with failures):\n{}",
            lines.len(),
            lines.join("\n")
        );
    }
}
