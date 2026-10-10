//! Theme picker (`T`) list: AIW themes first, grouped into Dark and Light
//! sections, with families collapsed into groups as upstream does.

use crate::app::ThemePickerEntry;
use crate::theme::{Theme, ThemeName};

fn is_light(name: ThemeName) -> bool {
    Theme::load(&name).is_light()
}

pub fn entries() -> Vec<ThemePickerEntry> {
    use ThemeName::*;

    let gruvbox_material = [
        GruvboxMaterialDarkHard,
        GruvboxMaterialDarkMedium,
        GruvboxMaterialDarkSoft,
        GruvboxMaterialMixDarkHard,
        GruvboxMaterialMixDarkMedium,
        GruvboxMaterialMixDarkSoft,
        GruvboxMaterialOriginalDarkHard,
        GruvboxMaterialOriginalDarkMedium,
        GruvboxMaterialOriginalDarkSoft,
        GruvboxMaterialLightHard,
        GruvboxMaterialLightMedium,
        GruvboxMaterialLightSoft,
        GruvboxMaterialMixLightHard,
        GruvboxMaterialMixLightMedium,
        GruvboxMaterialMixLightSoft,
        GruvboxMaterialOriginalLightHard,
        GruvboxMaterialOriginalLightMedium,
        GruvboxMaterialOriginalLightSoft,
    ];
    let material = |light: bool| ThemePickerEntry::Group {
        label: "Gruvbox Material",
        themes: gruvbox_material
            .iter()
            .copied()
            .filter(|name| is_light(*name) == light)
            .collect(),
    };

    vec![
        ThemePickerEntry::Header("Dark"),
        ThemePickerEntry::Theme(Ember),
        ThemePickerEntry::Theme(Graphite),
        ThemePickerEntry::Theme(Midnight),
        ThemePickerEntry::Theme(Default),
        ThemePickerEntry::Theme(Classic),
        ThemePickerEntry::Theme(Dracula),
        ThemePickerEntry::Theme(Nord),
        ThemePickerEntry::Theme(GruvboxDark),
        ThemePickerEntry::Group {
            label: "Catppuccin",
            themes: vec![CatppuccinFrappe, CatppuccinMacchiato, CatppuccinMocha],
        },
        material(false),
        ThemePickerEntry::Header("Light"),
        ThemePickerEntry::Theme(Paper),
        ThemePickerEntry::Theme(Porcelain),
        ThemePickerEntry::Theme(GruvboxLight),
        ThemePickerEntry::Theme(CatppuccinLatte),
        material(true),
    ]
}

/// The next selectable row from `from` in direction `step` (+1 / -1),
/// skipping section headers. Stays put at either end.
pub fn step_selection(entries: &[ThemePickerEntry], from: usize, step: isize) -> usize {
    let mut i = from as isize;
    loop {
        i += step;
        if i < 0 || i as usize >= entries.len() {
            return from;
        }
        if !matches!(entries[i as usize], ThemePickerEntry::Header(_)) {
            return i as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(entry: &ThemePickerEntry) -> Vec<ThemeName> {
        match entry {
            ThemePickerEntry::Theme(name) => vec![*name],
            ThemePickerEntry::Group { themes, .. } => themes.clone(),
            ThemePickerEntry::Header(_) => vec![],
        }
    }

    #[test]
    fn every_theme_is_listed_once_in_the_right_section() {
        let entries = entries();
        let mut seen = Vec::new();
        let mut light_section = None;
        for entry in &entries {
            if let ThemePickerEntry::Header(label) = entry {
                light_section = Some(*label == "Light");
                continue;
            }
            for name in members(entry) {
                assert_eq!(
                    Some(is_light(name)),
                    light_section,
                    "{name} is in the wrong section"
                );
                seen.push(name);
            }
        }
        let mut all = Theme::list();
        all.sort_by_key(|n| n.to_string());
        seen.sort_by_key(|n| n.to_string());
        assert_eq!(seen, all);
    }

    #[test]
    fn navigation_skips_headers() {
        let entries = entries();
        let light_header = entries
            .iter()
            .position(|e| matches!(e, ThemePickerEntry::Header("Light")))
            .unwrap();
        assert_eq!(
            step_selection(&entries, light_header - 1, 1),
            light_header + 1
        );
        assert_eq!(
            step_selection(&entries, light_header + 1, -1),
            light_header - 1
        );
        // The first row is a header: moving up from the first theme stays.
        assert_eq!(step_selection(&entries, 1, -1), 1);
        let last = entries.len() - 1;
        assert_eq!(step_selection(&entries, last, 1), last);
    }
}
