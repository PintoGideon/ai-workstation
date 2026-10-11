use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, AppMode, DiffViewerFocus};

const PATCH_SCROLL_STEP: usize = 1;
/// How far Ctrl+J/K (or Ctrl+↓/↑) moves, for covering ground faster than `j`/`k`.
const PATCH_JUMP_STEP: usize = 10;
const PATCH_PAGE_STEP: usize = 20;

pub fn handle_diff_picker_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.close_diff_picker(),
        KeyCode::Down | KeyCode::Char('j') => app.diff_picker_select_next(),
        KeyCode::Up | KeyCode::Char('k') => app.diff_picker_select_prev(),
        KeyCode::Enter => app.diff_picker_choose(),
        _ => {}
    }
    Ok(())
}

/// Lines a `j`/`k` press moves: one, or [`PATCH_JUMP_STEP`] with Ctrl held.
pub(crate) fn scroll_step(key: &KeyEvent) -> usize {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        PATCH_JUMP_STEP
    } else {
        PATCH_SCROLL_STEP
    }
}

/// Walk the selection file by file (not tree row by tree row) until it stops
/// moving, so `g`/`G` in the file list reach the first / last file.
fn walk_files(app: &mut App, forward: bool) {
    while matches!(app.diff_viewer_focus(), Some(DiffViewerFocus::FileList)) {
        let before = match &app.mode {
            AppMode::DiffViewer(state) => state.selected_file,
            _ => break,
        };
        if forward {
            app.diff_viewer_select_next_file();
        } else {
            app.diff_viewer_select_prev_file();
        }
        let after = match &app.mode {
            AppMode::DiffViewer(state) => state.selected_file,
            _ => break,
        };
        if after == before {
            break;
        }
    }
}

pub fn handle_diff_viewer_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let code = key.code;
    let step = scroll_step(&key);

    // The base-ref prompt is a single-line text input that takes precedence over
    // every other binding while it is open.
    let editing_base_ref =
        matches!(&app.mode, AppMode::DiffViewer(state) if state.editing_base_ref);
    if editing_base_ref {
        match code {
            KeyCode::Enter => app.diff_viewer_submit_base_ref(),
            KeyCode::Esc => app.diff_viewer_cancel_base_ref(),
            KeyCode::Backspace => app.diff_viewer_base_ref_backspace(),
            KeyCode::Char(c) => app.diff_viewer_base_ref_input(c),
            _ => {}
        }
        return Ok(());
    }

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.close_diff_viewer();
        }
        KeyCode::Tab => {
            app.diff_viewer_toggle_focus();
        }
        KeyCode::Char('v') => {
            app.diff_viewer_toggle_layout();
        }
        KeyCode::Char('r') => {
            app.refresh_diff_viewer();
        }
        KeyCode::Char('b') => {
            app.diff_viewer_start_base_ref_edit();
        }
        KeyCode::Char('i') => {
            app.open_syntax_language_picker_for_selected_diff_file();
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            app.diff_viewer_expand_context();
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            app.diff_viewer_collapse_context();
        }
        KeyCode::Char('*') => {
            app.diff_viewer_toggle_whole_file_context();
        }
        KeyCode::Char('W') => {
            app.diff_viewer_toggle_ignore_whitespace();
        }
        // n/p move file-to-file regardless of focus; in the file list, j/k
        // walk *tree rows* (directories included).
        KeyCode::Char('n') => {
            app.diff_viewer_select_next_file();
        }
        KeyCode::Char('p') => {
            app.diff_viewer_select_prev_file();
        }
        KeyCode::Char('j') | KeyCode::Down => match app.diff_viewer_focus() {
            Some(DiffViewerFocus::FileList) => app.diff_viewer_tree_move(1),
            Some(DiffViewerFocus::Patch) => app.diff_viewer_scroll_patch_down(step),
            None => {}
        },
        KeyCode::Char('k') | KeyCode::Up => match app.diff_viewer_focus() {
            Some(DiffViewerFocus::FileList) => app.diff_viewer_tree_move(-1),
            Some(DiffViewerFocus::Patch) => app.diff_viewer_scroll_patch_up(step),
            None => {}
        },
        // Tree folding is a file-list concern only, so it can't steal keys from
        // the patch panel.
        KeyCode::Char('z') | KeyCode::Enter
            if app.diff_viewer_focus() == Some(DiffViewerFocus::FileList) =>
        {
            app.diff_viewer_tree_toggle_collapsed();
        }
        KeyCode::Char('Z') if app.diff_viewer_focus() == Some(DiffViewerFocus::FileList) => {
            app.diff_viewer_tree_toggle_all();
        }
        KeyCode::Char('h') | KeyCode::Left
            if app.diff_viewer_focus() == Some(DiffViewerFocus::FileList) =>
        {
            app.diff_viewer_tree_collapse_or_parent();
        }
        KeyCode::Char('l') | KeyCode::Right
            if app.diff_viewer_focus() == Some(DiffViewerFocus::FileList) =>
        {
            app.diff_viewer_tree_expand();
        }
        KeyCode::PageDown => {
            app.diff_viewer_scroll_patch_down(PATCH_PAGE_STEP);
        }
        KeyCode::PageUp => {
            app.diff_viewer_scroll_patch_up(PATCH_PAGE_STEP);
        }
        KeyCode::Char('g') => match app.diff_viewer_focus() {
            Some(DiffViewerFocus::FileList) => walk_files(app, false),
            Some(DiffViewerFocus::Patch) => app.diff_viewer_scroll_patch_top(),
            None => {}
        },
        KeyCode::Char('G') => match app.diff_viewer_focus() {
            Some(DiffViewerFocus::FileList) => walk_files(app, true),
            Some(DiffViewerFocus::Patch) => app.diff_viewer_scroll_patch_bottom(),
            None => {}
        },
        _ => {}
    }

    Ok(())
}

/// Scrolls the patch by `lines`. Does nothing over the base-ref prompt, where
/// a stray scroll would be surprising.
pub fn handle_diff_viewer_wheel(app: &mut App, lines: usize, down: bool) {
    let scrollable = matches!(&app.mode, AppMode::DiffViewer(state) if !state.editing_base_ref);
    if !scrollable {
        return;
    }
    if down {
        app.diff_viewer_scroll_patch_down(lines);
    } else {
        app.diff_viewer_scroll_patch_up(lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppMode, DiffViewerLayout, DiffViewerState, ViewState};
    use crate::diff::{DiffFile, DiffFileStatus};
    use crate::project::{ProjectStore, VibeMode};
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn make_viewer_app(workdir: &Path, paths: &[&str]) -> App {
        let mut app = crate::app::App::new_for_test(
            ProjectStore {
                version: 5,
                projects: vec![],
                session_bookmarks: vec![],
                available_harnesses: vec![],
                prompt_templates: Vec::new(),
                extra: HashMap::new(),
            },
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        let mut state = DiffViewerState::new(
            ViewState::new(
                "proj".into(),
                "feat".into(),
                "sess".into(),
                "claude".into(),
                "Claude".into(),
                crate::project::SessionKind::Claude,
                VibeMode::Vibeless,
            ),
            workdir.to_path_buf(),
        );
        state.files = paths
            .iter()
            .map(|p| DiffFile {
                old_path: Some((*p).to_string()),
                path: (*p).to_string(),
                status: DiffFileStatus::Modified,
                additions: 1,
                deletions: 1,
                is_binary: false,
                old_content: None,
                new_content: None,
                patch: String::new(),
                hunks: vec![],
            })
            .collect();
        app.mode = AppMode::DiffViewer(state);
        app
    }

    /// A single-file viewer with `lines` added lines, focused on the patch.
    fn make_long_patch_app(workdir: &Path, lines: usize) -> App {
        let mut patch =
            format!("diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -0,0 +1,{lines} @@\n");
        for i in 0..lines {
            patch.push_str(&format!("+line {i}\n"));
        }
        let mut app = make_viewer_app(workdir, &["a.rs"]);
        let files = crate::diff::parse_unified_diff(&patch).unwrap();
        if let AppMode::DiffViewer(state) = &mut app.mode {
            state.files = files;
            state.focus = DiffViewerFocus::Patch;
        }
        app
    }

    fn patch_scroll(app: &App) -> usize {
        match &app.mode {
            AppMode::DiffViewer(state) => state.patch_scroll,
            _ => unreachable!(),
        }
    }

    fn selected(app: &App) -> usize {
        match &app.mode {
            AppMode::DiffViewer(s) => s.selected_file,
            _ => panic!("not in the diff viewer"),
        }
    }

    #[test]
    fn ctrl_j_and_k_scroll_the_patch_further_than_j_and_k() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_long_patch_app(dir.path(), 60);

        handle_diff_viewer_key(&mut app, ctrl(KeyCode::Char('j'))).unwrap();
        assert_eq!(patch_scroll(&app), PATCH_JUMP_STEP);
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('j'))).unwrap();
        assert_eq!(patch_scroll(&app), PATCH_JUMP_STEP + 1);
        handle_diff_viewer_key(&mut app, ctrl(KeyCode::Char('k'))).unwrap();
        assert_eq!(patch_scroll(&app), 1);
    }

    #[test]
    fn mouse_wheel_scrolls_the_patch() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_long_patch_app(dir.path(), 60);

        handle_diff_viewer_wheel(&mut app, 3, true);
        handle_diff_viewer_wheel(&mut app, 3, true);
        assert_eq!(patch_scroll(&app), 6);
        handle_diff_viewer_wheel(&mut app, 3, false);
        assert_eq!(patch_scroll(&app), 3);
    }

    #[test]
    fn mouse_wheel_does_nothing_while_typing_a_base_ref() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_long_patch_app(dir.path(), 60);
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('b'))).unwrap();

        handle_diff_viewer_wheel(&mut app, 3, true);
        assert_eq!(patch_scroll(&app), 0);
    }

    #[test]
    fn n_and_p_navigate_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_viewer_app(dir.path(), &["a.rs", "b.rs"]);

        assert_eq!(selected(&app), 0);
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('n'))).unwrap();
        assert_eq!(selected(&app), 1);
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('n'))).unwrap();
        assert_eq!(selected(&app), 1, "n stops at the last file");
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('p'))).unwrap();
        assert_eq!(selected(&app), 0);
    }

    #[test]
    fn g_and_shift_g_jump_to_the_first_and_last_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_viewer_app(dir.path(), &["a.rs", "b.rs", "c.rs"]);

        handle_diff_viewer_key(&mut app, key(KeyCode::Char('G'))).unwrap();
        assert_eq!(selected(&app), 2);
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('g'))).unwrap();
        assert_eq!(selected(&app), 0);
    }

    #[test]
    fn base_ref_prompt_opens_types_and_submits_into_override() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_viewer_app(dir.path(), &["a.rs"]);

        handle_diff_viewer_key(&mut app, key(KeyCode::Char('b'))).unwrap();
        assert!(matches!(&app.mode, AppMode::DiffViewer(s) if s.editing_base_ref));

        for c in "develop".chars() {
            handle_diff_viewer_key(&mut app, key(KeyCode::Char(c))).unwrap();
        }
        handle_diff_viewer_key(&mut app, key(KeyCode::Backspace)).unwrap(); // -> "develo"

        // Enter submits: the override is recorded and the viewer re-enters
        // loading so the diff reloads against it.
        handle_diff_viewer_key(&mut app, key(KeyCode::Enter)).unwrap();
        match &app.mode {
            AppMode::DiffViewerLoading(s) => {
                assert_eq!(s.override_base_ref.as_deref(), Some("develo"));
                assert!(!s.editing_base_ref);
            }
            _ => panic!("expected loading state after submit"),
        }
    }

    #[test]
    fn base_ref_prompt_escape_cancels_without_override() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_viewer_app(dir.path(), &["a.rs"]);

        handle_diff_viewer_key(&mut app, key(KeyCode::Char('b'))).unwrap();
        for c in "main".chars() {
            handle_diff_viewer_key(&mut app, key(KeyCode::Char(c))).unwrap();
        }
        handle_diff_viewer_key(&mut app, key(KeyCode::Esc)).unwrap();

        match &app.mode {
            AppMode::DiffViewer(s) => {
                assert!(!s.editing_base_ref);
                assert!(s.override_base_ref.is_none());
                assert!(s.base_ref_input.is_empty());
            }
            _ => panic!("expected diff viewer"),
        }
    }

    #[test]
    fn base_ref_prompt_blank_submit_clears_override() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_viewer_app(dir.path(), &["a.rs"]);
        if let AppMode::DiffViewer(state) = &mut app.mode {
            state.override_base_ref = Some("develop".into());
        }

        // Open (pre-fills with the current override), clear it, submit blank.
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('b'))).unwrap();
        for _ in 0.."develop".len() {
            handle_diff_viewer_key(&mut app, key(KeyCode::Backspace)).unwrap();
        }
        handle_diff_viewer_key(&mut app, key(KeyCode::Enter)).unwrap();

        match &app.mode {
            AppMode::DiffViewerLoading(s) => assert!(s.override_base_ref.is_none()),
            _ => panic!("expected loading state after submit"),
        }
    }

    #[test]
    fn i_opens_syntax_picker_for_selected_diff_file() {
        let mut app = make_viewer_app(&PathBuf::from("/tmp/project"), &["src/main.rs"]);
        if let AppMode::DiffViewer(state) = &mut app.mode {
            state.layout = DiffViewerLayout::Unified;
        }

        handle_diff_viewer_key(&mut app, key(KeyCode::Char('i'))).unwrap();

        match &app.mode {
            AppMode::SyntaxLanguagePicker(state) => {
                assert_eq!(
                    state.languages[state.selected].language,
                    crate::highlight::HighlightLanguage::Rust
                );
            }
            other => panic!(
                "expected syntax picker, got {:?}",
                std::mem::discriminant(other)
            ),
        }
    }

    #[test]
    fn file_list_j_k_walk_tree_rows_and_z_folds_a_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_viewer_app(dir.path(), &["README.md", "src/a.rs", "src/b.rs"]);

        // Rows: README.md, dir src, a.rs, b.rs. j from the first file lands on
        // the directory header without changing the diffed file.
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('j'))).unwrap();
        let state = match &app.mode {
            AppMode::DiffViewer(state) => state,
            _ => panic!("left the diff viewer"),
        };
        assert_eq!(state.tree_cursor_dir.as_deref(), Some("src"));
        assert_eq!(state.selected_file, 0);

        // z folds it; the two files beneath lose their rows.
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('z'))).unwrap();
        let state = match &app.mode {
            AppMode::DiffViewer(state) => state,
            _ => panic!("left the diff viewer"),
        };
        assert!(state.collapsed_dirs.contains("src"));
        assert_eq!(state.file_tree_rows().len(), 2);

        // n keeps its file meaning and re-expands what it lands inside.
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('n'))).unwrap();
        let state = match &app.mode {
            AppMode::DiffViewer(state) => state,
            _ => panic!("left the diff viewer"),
        };
        assert_eq!(state.selected_file, 1);
        assert!(state.collapsed_dirs.is_empty());
    }

    #[test]
    fn tree_fold_keys_are_ignored_while_the_patch_is_focused() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_viewer_app(dir.path(), &["src/a.rs"]);
        handle_diff_viewer_key(&mut app, key(KeyCode::Tab)).unwrap();
        assert_eq!(app.diff_viewer_focus(), Some(DiffViewerFocus::Patch));

        handle_diff_viewer_key(&mut app, key(KeyCode::Char('z'))).unwrap();
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('Z'))).unwrap();

        let state = match &app.mode {
            AppMode::DiffViewer(state) => state,
            _ => panic!("left the diff viewer"),
        };
        assert!(
            state.collapsed_dirs.is_empty(),
            "folding belongs to the file list, not the patch panel"
        );
    }

    #[test]
    fn the_file_tree_groups_by_directory_and_folds() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut app = make_viewer_app(dir.path(), &["README.md", "src/a.rs", "src/ui/b.rs"]);

        let rows = match &app.mode {
            AppMode::DiffViewer(state) => state.file_tree_rows(),
            _ => panic!("left the diff viewer"),
        };
        assert_eq!(
            rows.len(),
            5,
            "expected README + src/ + a.rs + ui/ + b.rs: {rows:?}"
        );

        // j walks onto the `src` directory row, z folds it.
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('j'))).unwrap();
        handle_diff_viewer_key(&mut app, key(KeyCode::Char('z'))).unwrap();
        let state = match &app.mode {
            AppMode::DiffViewer(state) => state,
            _ => panic!("left the diff viewer"),
        };
        assert!(state.collapsed_dirs.contains("src"));
        assert_eq!(state.file_tree_rows().len(), 2);
        assert_eq!(
            state.selected_file, 0,
            "folding must not change the previewed file"
        );
    }
}
