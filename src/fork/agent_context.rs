//! FORK: the launch context in the agent's system prompt (item 6, DESIGN.md
//! §16): project, feature, branch, worktree and AIW session id, plus one
//! line asking for the branch name in commits and PR titles.
//!
//! - **Claude**: appended to `--append-system-prompt` (after the screenshot
//!   evidence text when there is one).
//! - **Codex**: appended to `developer_instructions` (passed with `-c`).
//! - **Gemini**: the `SessionStart` hook prints
//!   `hookSpecificOutput.additionalContext`.
//! - **Cursor**: the `sessionStart` hook prints `additional_context`.
//!
//! The hooks run `aiw session-context`, which rebuilds the paragraph from
//! the store (read-only) using the `AIW_FEATURE_SESSION_ID` the hook
//! inherited. `agent_context_prompt: false` turns all of it off.

use std::path::Path;

use serde_json::{Value, json};

use super::launch::LaunchContext;

/// The paragraph's first words, so tests and the e2e can find it.
pub const MARKER: &str = "AIW context:";

/// The paragraph for `ctx`.
pub fn context_paragraph(ctx: &LaunchContext) -> String {
    format!(
        "{MARKER} project {project}, feature {feature}, branch {branch}, {checkout} {path}, AIW session {id}. \
Mention the branch name in commit messages and PR titles.",
        project = ctx.project_name,
        feature = ctx.feature_name,
        branch = ctx.branch,
        checkout = if ctx.is_worktree {
            "worktree"
        } else {
            "checkout"
        },
        path = ctx.workdir.display(),
        id = ctx.feature_session_id,
    )
}

/// `args` with the paragraph appended to Claude's `--append-system-prompt`
/// (joined to an existing one, so Claude gets one flag).
pub fn claude_args(ctx: &LaunchContext, mut args: Vec<String>) -> Vec<String> {
    let paragraph = context_paragraph(ctx);
    if let Some(index) = args.iter().position(|a| a == "--append-system-prompt")
        && let Some(existing) = args.get_mut(index + 1)
    {
        existing.push_str("\n\n");
        existing.push_str(&paragraph);
        return args;
    }
    args.push("--append-system-prompt".to_string());
    args.push(paragraph);
    args
}

/// `args` with the paragraph appended to Codex's `developer_instructions`
/// (merged with the user's config layers and any earlier `-c` value).
pub fn codex_args(ctx: &LaunchContext, args: Vec<String>) -> Vec<String> {
    let paragraph = context_paragraph(ctx);
    crate::codex_config::with_screenshot_guidance(&ctx.workdir, &paragraph, args.clone())
        .unwrap_or(args)
}

/// What the Gemini / Cursor `SessionStart` hook prints.
pub fn session_start_json(harness: &str, paragraph: &str) -> Option<Value> {
    match harness.trim() {
        "gemini" => Some(json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": paragraph
            }
        })),
        "cursor" | "cursor-agent" | "agent" => Some(json!({ "additional_context": paragraph })),
        _ => None,
    }
}

/// The launch context of the feature session `session_id` in `store`.
pub fn context_from_store(
    store: &crate::project::ProjectStore,
    session_id: &str,
) -> Option<LaunchContext> {
    for project in &store.projects {
        for feature in &project.features {
            if feature.sessions.iter().any(|s| s.id == session_id) {
                return Some(LaunchContext::for_feature(
                    &project.name,
                    &project.repo,
                    feature,
                    session_id,
                ));
            }
        }
    }
    None
}

/// The paragraph for the session a hook runs in, from the database at
/// `db_path` (read-only). `None` when the session is unknown.
pub fn paragraph_for_session(db_path: &Path, session_id: &str) -> Option<String> {
    let db = crate::db::AiwDb::open_read_only(db_path).ok()?;
    let store = db.load_store().ok()?;
    context_from_store(&store, session_id).map(|ctx| context_paragraph(&ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{AgentKind, Feature, VibeMode};
    use std::path::PathBuf;

    fn ctx() -> LaunchContext {
        let feature = Feature::new(
            "hx-12".into(),
            "feature/hx-12".into(),
            PathBuf::from("/wt/hx-12"),
            true,
            VibeMode::Vibe,
            false,
            AgentKind::Claude,
            false,
        );
        LaunchContext::for_feature("RS", Path::new("/repo"), &feature, "fs-1")
    }

    #[test]
    fn the_paragraph_names_everything_and_asks_for_the_branch() {
        let text = context_paragraph(&ctx());
        assert!(text.starts_with("AIW context: project RS, feature hx-12, branch feature/hx-12, worktree /wt/hx-12, AIW session fs-1."));
        assert!(text.ends_with("Mention the branch name in commit messages and PR titles."));
    }

    #[test]
    fn claude_gets_one_append_flag_after_the_screenshot_text() {
        let args = claude_args(&ctx(), vec!["--chrome".into()]);
        assert_eq!(args[1], "--append-system-prompt");
        assert!(args[2].starts_with(MARKER));
        let merged = claude_args(
            &ctx(),
            vec![
                "--append-system-prompt".into(),
                "AIW screenshot evidence: …".into(),
            ],
        );
        assert_eq!(merged.len(), 2);
        assert!(merged[1].starts_with("AIW screenshot evidence: …\n\nAIW context:"));
    }

    #[test]
    fn codex_gets_the_paragraph_in_developer_instructions() {
        let args = codex_args(&ctx(), vec!["-C".into(), "/wt/hx-12".into()]);
        let value = args
            .windows(2)
            .find(|pair| pair[0] == "-c" && pair[1].starts_with("developer_instructions="))
            .map(|pair| pair[1].clone())
            .expect("a developer_instructions override");
        assert!(value.contains("AIW context: project RS"), "{value}");
    }

    #[test]
    fn session_start_json_matches_gemini_and_cursor() {
        let gemini = session_start_json("gemini", "ctx").unwrap();
        assert_eq!(
            gemini["hookSpecificOutput"]["hookEventName"],
            "SessionStart"
        );
        assert_eq!(gemini["hookSpecificOutput"]["additionalContext"], "ctx");
        let cursor = session_start_json("cursor", "ctx").unwrap();
        assert_eq!(cursor["additional_context"], "ctx");
        assert!(session_start_json("claude", "ctx").is_none());
    }

    #[test]
    fn the_paragraph_can_be_rebuilt_from_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("aiw.db");
        let db = crate::db::AiwDb::open(&db_path).unwrap();
        let mut feature = Feature::new(
            "hx-12".into(),
            "feature/hx-12".into(),
            PathBuf::from("/wt/hx-12"),
            true,
            VibeMode::Vibe,
            false,
            AgentKind::Gemini,
            false,
        );
        feature.add_session(crate::project::SessionKind::Gemini);
        let session_id = feature.sessions[0].id.clone();
        let mut project = crate::project::Project::new(
            "RS".into(),
            PathBuf::from("/repo"),
            true,
            AgentKind::Gemini,
        );
        project.features.push(feature);
        let store = crate::project::ProjectStore {
            version: 5,
            projects: vec![project],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: Default::default(),
        };
        db.save_store(&store).unwrap();
        drop(db);
        let text = paragraph_for_session(&db_path, &session_id).unwrap();
        assert!(text.contains("feature hx-12, branch feature/hx-12"));
        assert!(paragraph_for_session(&db_path, "nope").is_none());
    }
}
