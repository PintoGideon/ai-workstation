pub use crate::app::diff_state::*;
use ratatui_explorer::FileExplorer;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};

use crate::editor::TextEditor;
use crate::extension::{CustomSessionConfig, FeaturePreset, LifecycleHooks};
use crate::project::{AgentKind, SessionKind, VibeMode};
use crate::worktree::WorktreeInfo;

#[derive(Debug, Clone, PartialEq)]
pub enum ForkFeatureStep {
    Branch,
    Agent,
}

pub struct ForkFeatureState {
    pub source_pi: usize,
    pub source_fi: usize,
    pub project_name: String,
    pub project_repo: PathBuf,
    pub source_branch: String,
    pub new_branch: String,
    pub step: ForkFeatureStep,
    pub agent: AgentKind,
    pub agent_index: usize,
    pub mode: VibeMode,
    pub enable_chrome: bool,
    pub include_context: bool,
    /// FORK: §19.2 — the new feature's agent continues this conversation
    /// (`f` → "new worktree") instead of starting fresh.
    pub fork_conversation: Option<crate::fork::session_identity::ForkSource>,
}

#[derive(Debug, Clone)]
pub enum Selection {
    Project(usize),
    Feature(usize, usize),
    Session(usize, usize, usize),
}

#[derive(Clone)]
pub struct ViewState {
    pub project_name: String,
    pub feature_name: String,
    pub session: String,
    pub window: String,
    pub session_label: String,
    pub session_kind: SessionKind,
    pub vibe_mode: VibeMode,
}

impl ViewState {
    // Constructor args map 1:1 onto the identity fields of ViewState.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project_name: String,
        feature_name: String,
        session: String,
        window: String,
        session_label: String,
        session_kind: SessionKind,
        vibe_mode: VibeMode,
    ) -> Self {
        Self {
            project_name,
            feature_name,
            session,
            window,
            session_label,
            session_kind,
            vibe_mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PendingInput {
    pub session_id: String,
    pub cwd: String,
    pub message: String,
    pub notification_type: String,
    pub file_path: PathBuf,
    pub target_file_path: Option<String>,
    pub relative_path: Option<String>,
    pub change_id: Option<String>,
    pub tool: Option<String>,
    pub old_snippet: Option<String>,
    pub new_snippet: Option<String>,
    pub original_file: Option<String>,
    pub proposed_file: Option<String>,
    pub is_new_file: Option<bool>,
    pub reason: Option<String>,
    pub response_file: Option<String>,
    pub project_name: Option<String>,
    pub feature_name: Option<String>,
    pub proceed_signal: Option<String>,
    pub request_id: Option<String>,
    pub reply_socket: Option<String>,
}

impl PendingInput {
    /// Whether this notification describes the *session* as having stopped and
    /// being blocked on the user, rather than a discrete piece of work to act
    /// on.
    ///
    /// The distinction is what makes deduplication safe. A session can only be
    /// stopped once, so every report of it is the same standing fact restated:
    /// Claude's Stop hook fires at every turn boundary, Codex's notify hook on
    /// every prompt, and `sync.rs` infers the same stop again whenever a
    /// thinking marker drops. A diff review or a change reason is the
    /// opposite — each one is its own request about its own edit, and they
    /// must keep queueing one row apiece.
    pub fn is_session_wait(&self) -> bool {
        matches!(self.notification_type.as_str(), "input-request" | "stop")
    }

    /// Whether `other` is a re-report of the same session's stop, and so
    /// replaces this entry instead of queueing beside it.
    ///
    /// Identity is the feature, which is the granularity everything else in
    /// AIW already uses for this: a feature owns one tmux session, the
    /// attention layer is keyed by it, and `sync.rs` clears pending inputs by
    /// it. `session_id` is only the fallback for a notification that could not
    /// be resolved to a feature — it holds the AIW tmux session when the hook
    /// environment supplied one and the harness's own session id otherwise, so
    /// two reports of one stop can disagree about it.
    pub fn is_same_session_wait(&self, other: &PendingInput) -> bool {
        if !self.is_session_wait() || !other.is_session_wait() {
            return false;
        }
        match (self.feature_name.as_deref(), other.feature_name.as_deref()) {
            (Some(mine), Some(theirs)) => self.project_name == other.project_name && mine == theirs,
            _ => !self.session_id.is_empty() && self.session_id == other.session_id,
        }
    }
}

/// A request to suspend the TUI and hand the terminal to `$VISUAL`/`$EDITOR`.
pub enum RenameReturnTo {
    Dashboard,
}

pub struct RenameSessionState {
    pub project_idx: usize,
    pub feature_idx: usize,
    pub session_idx: usize,
    pub input: String,
    pub return_to: RenameReturnTo,
}

#[derive(Clone)]
pub enum NewSessionTarget {
    Builtin(SessionKind),
    // Boxed: CustomSessionConfig is ~10 fields and would dominate the enum size.
    Custom(Box<CustomSessionConfig>),
}

#[derive(Clone)]
pub struct NewSessionNameState {
    pub project_idx: usize,
    pub feature_idx: usize,
    pub target: NewSessionTarget,
    pub input: String,
    pub return_to: SessionPickerState,
}

#[derive(Debug, Clone)]
pub struct RenameFeatureState {
    pub project_idx: usize,
    pub feature_idx: usize,
    pub input: String,
}

/// Which field of [`ContextSettingsState`] currently has input focus.
/// Declared in edit order so `next()`/`prev()` can wrap with simple
/// arithmetic instead of a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextSettingsField {
    WindowLimit,
    WarningPercent,
    CriticalPercent,
}

impl ContextSettingsField {
    const ALL: [Self; 3] = [
        Self::WindowLimit,
        Self::WarningPercent,
        Self::CriticalPercent,
    ];

    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let index = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Global context-window/severity settings dialog (`w` on the dashboard).
/// Edits [`super::AppConfig::context_window_override`],
/// `context_warning_percent`, and `context_critical_percent` directly —
/// unlike `ConfigWizard` this has no project/global scope choice, since the
/// values it edits are process-wide by design.
pub struct ContextSettingsState {
    pub field: ContextSettingsField,
    /// Empty means "no override" (falls back to each harness's own default).
    pub window_limit_input: String,
    pub warning_input: String,
    pub critical_input: String,
    pub error: Option<String>,
}

pub struct SessionConfigState {
    pub project_idx: usize,
    pub feature_idx: usize,
    pub project_name: String,
    pub feature_name: String,
    pub current_agent: AgentKind,
    pub allowed_agents: Vec<AgentKind>,
    pub selected_agent: usize,
}

pub struct ProjectAgentConfigState {
    pub project_idx: usize,
    pub project_name: String,
    pub current_agent: AgentKind,
    pub allowed_agents: Vec<AgentKind>,
    pub selected_agent: usize,
}

#[derive(Debug, Clone)]
pub struct ClaudeSessionPickerState {
    pub sessions: Vec<super::claude_sessions::ClaudeSessionInfo>,
    pub selected: usize,
    pub workdir: PathBuf,
    /// The `FeatureSession::id` this picker was opened from (or, when opened
    /// from the feature row rather than a specific session, the feature's
    /// first claude-kind session) — the *only* session a restore may
    /// overwrite. Every other claude session in the feature keeps its own
    /// saved id. `None` when the feature had no sessions at all yet (they
    /// are created fresh on restore, so the lone one created is the target).
    pub target_session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CodexSessionPickerState {
    pub sessions: Vec<super::codex_sessions::CodexSessionInfo>,
    pub selected: usize,
    pub workdir: PathBuf,
    /// The `FeatureSession::id` this picker was opened from (or, when opened
    /// from the feature row rather than a specific session, the feature's
    /// first codex-kind session) — the *only* session a restore may
    /// overwrite. Every other codex session in the feature keeps its own
    /// saved id. `None` when the feature had no sessions at all yet (they
    /// are created fresh on restore, so the lone one created is the target).
    pub target_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoppedSessionChoice {
    Resume,
    Clear,
    /// Hand off to the harness's saved-transcript picker (the `S` path), so
    /// older sessions than the one AIW has recorded stay reachable.
    PickSession,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct StoppedSessionDialogState {
    pub project_id: String,
    pub feature_id: String,
    pub session_id: String,
    pub selected: usize,
    /// Choices offered for this session, in display order. Only harnesses with
    /// a transcript picker get [`StoppedSessionChoice::PickSession`]; every
    /// entry present is selectable, so there is no disabled state to skip.
    pub choices: Vec<StoppedSessionChoice>,
    /// Harness name used in the dialog copy ("Claude", "Codex", ...).
    pub harness_label: String,
}

#[derive(Clone)]
pub struct BookmarkPickerState {
    pub selected: usize,
    pub from_view: Option<ViewState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffViewerFocus {
    FileList,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiffViewerLayout {
    Unified,
    SideBySide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffScope {
    /// The existing branch snapshot: every commit since the resolved base plus
    /// staged, unstaged, and untracked worktree changes.
    CurrentChanges,
    /// Exactly one commit, compared with its first parent.
    Commit(crate::diff::DiffCommit),
}

pub struct DiffPickerState {
    pub from_view: ViewState,
    pub workdir: PathBuf,
    pub commits: Vec<crate::diff::DiffCommit>,
    /// Zero is "all current changes"; commit rows start at one.
    pub selected: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ComposeCommandEntry {
    /// Command name without the leading slash (e.g. "compact").
    pub name: String,
    pub description: String,
}

/// One row in the `Latest Prompt` recall list: either a prompt actually sent
/// to the harness (scanned from its transcript) or one AIW computed to seed a
/// session but couldn't deliver, stashed by
/// [`crate::app::App::stash_lost_prompt`] when the launch that would have
/// carried it failed. Kept as a separate variant rather than folded into
/// [`crate::app::util::PromptEntry`] so an unsent row can carry *why* it's
/// here and be cleared from `unsent_prompts` once picked, without touching
/// the transcript-scan path at all.
#[derive(Clone)]
pub enum LatestPromptItem {
    Sent(crate::app::util::PromptEntry),
    Unsent(crate::db::unsent_prompts::UnsentPrompt),
}

impl LatestPromptItem {
    pub fn text(&self) -> &str {
        match self {
            LatestPromptItem::Sent(entry) => &entry.text,
            LatestPromptItem::Unsent(prompt) => &prompt.body,
        }
    }

    pub fn is_unsent(&self) -> bool {
        matches!(self, LatestPromptItem::Unsent(_))
    }
}

#[derive(Clone)]
pub struct LatestPromptState {
    pub view: ViewState,
    /// Unsent entries for this checkout are prepended ahead of the scanned
    /// transcript history — see `App::poll_latest_prompt_menu_bg`.
    pub prompts: Vec<LatestPromptItem>,
    pub selected: usize,
}

/// A prompt-library row: a template plus where it came from. Phase 1
/// only surfaces `User` templates; the `source` field is ready for the
/// declarative `Global` / `Project` templates added in phase 3.
#[derive(Clone)]
pub struct PromptLibraryEntry {
    pub template: crate::prompt_library::PromptTemplate,
    pub source: crate::prompt_library::PromptSource,
    /// Resolved on-disk location this entry is read from / written to:
    /// the SQLite store for `User`, the relevant `aiw.json` for
    /// config sources. Filled in by `rebuild_prompt_library`; `None` when
    /// the scope has no resolvable location (no project context, or the
    /// empty test store path).
    pub source_path: Option<PathBuf>,
}

/// Picker over the merged, source-tagged prompt library. Mirrors the
/// `LatestPrompt` shape (a list with a `selected` index) plus fuzzy
/// filtering and an optional `from_view` to inject back into.
#[derive(Clone)]
pub struct PromptLibraryState {
    pub templates: Vec<PromptLibraryEntry>,
    /// Indices into `templates` matching `query`, best score first.
    pub filtered: Vec<usize>,
    pub query: String,
    pub search_active: bool,
    pub selected: usize,
    /// Set after the first `d` press; a second `d` confirms deletion.
    pub confirm_delete: bool,
    /// Set after `x`; `g` exports to global config, `p` to project.
    pub pending_export: bool,
    pub from_view: Option<ViewState>,
}

impl PromptLibraryState {
    /// The template currently highlighted in the filtered list.
    pub fn selected_entry(&self) -> Option<&PromptLibraryEntry> {
        self.filtered
            .get(self.selected)
            .and_then(|idx| self.templates.get(*idx))
    }
}

/// Which field of the prompt editor currently has focus. `Tab` cycles
/// Name → Tags → Body (and `Shift+Tab` the reverse). Name and Tags are
/// single-line text fields; Body is the multi-line `TextEditor`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PromptEditorFocus {
    Name,
    Tags,
    Body,
}

impl PromptEditorFocus {
    /// The next field in the Name → Tags → Body → Name cycle.
    pub fn next(self) -> Self {
        match self {
            PromptEditorFocus::Name => PromptEditorFocus::Tags,
            PromptEditorFocus::Tags => PromptEditorFocus::Body,
            PromptEditorFocus::Body => PromptEditorFocus::Name,
        }
    }

    /// The previous field in the cycle (reverse of `next`).
    pub fn prev(self) -> Self {
        match self {
            PromptEditorFocus::Name => PromptEditorFocus::Body,
            PromptEditorFocus::Tags => PromptEditorFocus::Name,
            PromptEditorFocus::Body => PromptEditorFocus::Tags,
        }
    }
}

/// Create/edit dialog for a user template. `editing_id` is `None` for a
/// new template. `return_to` is where to land after save/cancel — the
/// picker it was opened from, or the underlying `Viewing` mode when
/// saved straight from the compose box.
pub struct PromptEditorState {
    pub editing_id: Option<String>,
    /// Where the template lives; determines which store/file is written on save.
    pub editing_source: crate::prompt_library::PromptSource,
    /// Original template for config-file sources (Project/Global/Worktree),
    /// preserving id/description/placeholders across edits.
    pub original_template: Option<crate::prompt_library::PromptTemplate>,
    pub name: String,
    /// Raw comma/space-separated tag input; parsed into the template's
    /// `tags` on save (see `prompt_library::parse_tags`).
    pub tags: String,
    pub focus: PromptEditorFocus,
    pub editor: TextEditor,
    pub return_to: Box<AppMode>,
    /// Where a save will land, for the editor's destination hint: the
    /// SQLite store for `User`, the relevant `aiw.json` otherwise.
    /// `None` when unresolvable (no project context, or the test store).
    pub dest_path: Option<PathBuf>,
}

/// Collects a value for each `{{slot}}` in a template before injection.
/// One field is shown at a time (`current` of `placeholders.len()`); each
/// slot's value lives in `values` (seeded with its default) so moving
/// back and forth preserves edits. `from_view` is where the rendered
/// prompt is delivered once every field is filled.
pub struct PlaceholderFillState {
    pub template: crate::prompt_library::PromptTemplate,
    /// Slots to fill, in body order. Built by `resolve_placeholders`.
    pub placeholders: Vec<crate::prompt_library::PromptPlaceholder>,
    /// One entry per placeholder; seeded with defaults, updated on nav.
    pub values: Vec<String>,
    pub current: usize,
    /// Editor for the field currently shown; reseeded from `values` on nav.
    /// Unused while the active slot is a `Select` (the option list drives it).
    pub input: TextEditor,
    /// Highlighted option index for a `Select` slot; ignored otherwise.
    pub select_index: usize,
    /// Whether the user has turned vim on for the fill fields. Persisted on
    /// the state (not the editor) so the choice survives `enter()` rebuilding
    /// `input` when moving between slots. Only applies to multi-line slots.
    pub vim_enabled: bool,
    pub from_view: Option<ViewState>,
}

impl PlaceholderFillState {
    pub fn current_placeholder(&self) -> Option<&crate::prompt_library::PromptPlaceholder> {
        self.placeholders.get(self.current)
    }

    /// Whether the active field accepts newlines (Enter inserts a line break
    /// rather than advancing to the next slot).
    pub fn current_is_multiline(&self) -> bool {
        matches!(
            self.current_placeholder().map(|p| &p.kind),
            Some(crate::prompt_library::PlaceholderKind::MultiLine { .. })
        )
    }

    /// Whether the active slot is a `Select` (choose from a fixed option list).
    pub fn is_select(&self) -> bool {
        matches!(
            self.current_placeholder().map(|p| &p.kind),
            Some(crate::prompt_library::PlaceholderKind::Select { .. })
        )
    }

    /// The options for the active slot, or an empty slice when it isn't a
    /// `Select`.
    pub fn current_options(&self) -> &[String] {
        match self.current_placeholder().map(|p| &p.kind) {
            Some(crate::prompt_library::PlaceholderKind::Select { options }) => options.as_slice(),
            _ => &[],
        }
    }

    /// Move to slot `idx`: reseed the editor from its stored value and point
    /// `select_index` at that value's position in the options (0 otherwise).
    pub fn enter(&mut self, idx: usize) {
        self.current = idx;
        let value = self.values.get(idx).cloned().unwrap_or_default();
        let is_multiline = matches!(
            self.placeholders.get(idx).map(|p| &p.kind),
            Some(crate::prompt_library::PlaceholderKind::MultiLine { .. })
        );
        self.select_index = match self.placeholders.get(idx).map(|p| &p.kind) {
            Some(crate::prompt_library::PlaceholderKind::Select { options }) => {
                options.iter().position(|o| o == &value).unwrap_or(0)
            }
            _ => 0,
        };
        // Vim applies only to multi-line slots; single-line/select slots use
        // Enter to advance, so a plain editor keeps that behaviour intact.
        self.input = if self.vim_enabled && is_multiline {
            TextEditor::with_vim(value)
        } else {
            TextEditor::new(value)
        };
    }

    /// Toggle vim on the active multi-line field, remembering the choice for
    /// later slots. No-op (and reports `false`) on non-multi-line slots, where
    /// vim would hijack Enter's "advance field" behaviour.
    pub fn toggle_input_vim(&mut self) -> bool {
        if !self.current_is_multiline() {
            return false;
        }
        self.input.toggle_vim();
        self.vim_enabled = self.input.vim_mode().is_some();
        true
    }

    /// Record the active slot's value into `values`: the chosen option for a
    /// `Select`, the editor text otherwise.
    pub fn commit_current(&mut self) {
        let value = match self.current_placeholder().map(|p| &p.kind) {
            Some(crate::prompt_library::PlaceholderKind::Select { options }) => {
                options.get(self.select_index).cloned().unwrap_or_default()
            }
            _ => self.input.text().to_string(),
        };
        if let Some(slot) = self.values.get_mut(self.current) {
            *slot = value;
        }
    }

    /// Highlight the next option (wrapping) for a `Select` slot.
    pub fn select_next(&mut self) {
        let len = self.current_options().len();
        if len > 0 {
            self.select_index = (self.select_index + 1) % len;
        }
    }

    /// Highlight the previous option (wrapping) for a `Select` slot.
    pub fn select_prev(&mut self) {
        let len = self.current_options().len();
        if len > 0 {
            self.select_index = self.select_index.checked_sub(1).unwrap_or(len - 1);
        }
    }
}

pub struct HelpState {
    pub from_view: Option<ViewState>,
    pub scroll_offset: usize,
}

/// A search-as-you-type picker over the workspace's agent skills, launched
/// from a prompt-editing surface (the prompt editor body or a text fill
/// field). Selecting an entry inserts its `/skill-name` invocation at the
/// editor cursor; `return_to` holds the editing mode to restore afterwards.
pub struct SkillPickerState {
    /// All available skills (global + project), name-sorted.
    pub skills: Vec<ComposeCommandEntry>,
    /// Indices into `skills` matching the current query, best match first.
    pub filtered: Vec<usize>,
    pub query: String,
    pub selected: usize,
    /// The editing mode to return to on select/cancel — `PromptEditor` or
    /// `PlaceholderFill`. Boxed because `AppMode` is large.
    pub return_to: Box<AppMode>,
}

impl SkillPickerState {
    /// The currently highlighted skill, if any survive the filter.
    pub fn selected_skill(&self) -> Option<&ComposeCommandEntry> {
        self.filtered
            .get(self.selected)
            .and_then(|idx| self.skills.get(*idx))
    }
}

/// What an active TODOs inline edit targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoEditTarget {
    /// Adding a brand-new item (its title).
    New,
    /// Editing the selected item's title.
    Title,
    /// Editing the selected item's notes/detail body (multi-line).
    Notes,
    /// Editing the list's free-form scratchpad note (persisted in the
    /// legacy `carry_over` column).
    Scratchpad,
}

/// An in-progress inline edit within the TODOs overlay.
pub struct TodoEditor {
    pub target: TodoEditTarget,
    pub editor: TextEditor,
}

/// Which scope a pane of the TODOs overlay shows. The variants are in the
/// order the panes are laid out and the order ties between them resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoPaneKind {
    /// This feature's own checkout. Absent for a feature sitting on the repo
    /// root, which has no worktree of its own to scope a list to.
    Worktree,
    /// The project's list — the only scope that existed before scoping.
    Project,
    /// The machine-wide list, belonging to no project.
    Global,
}

impl TodoPaneKind {
    /// The pane header, and the noun used when a message has to name a scope.
    pub fn label(self) -> &'static str {
        match self {
            TodoPaneKind::Worktree => "Worktree",
            TodoPaneKind::Project => "Project",
            TodoPaneKind::Global => "Global",
        }
    }
}

/// One scope's list within the TODOs overlay: its own items, cursor, scroll,
/// and scratchpad, so switching focus never disturbs the pane being left.
pub struct TodoPane {
    pub kind: TodoPaneKind,
    /// The scope this pane reads and writes. Carries the project id and
    /// workdir, so it is the whole key to the pane's list.
    pub scope: crate::db::todos::TodoScope,
    /// Name shown in the pane header beside the scope label (the worktree's
    /// feature name, the project name, or nothing for global).
    pub title: String,
    /// The loaded list (scratchpad note, id). `None` until the list exists —
    /// with no DB (tests) it is synthesized on first write, and with one it is
    /// created lazily, so an untouched scope leaves no row behind.
    pub list: Option<crate::db::todos::TodoList>,
    /// Items in display order (open first, then by sort_order).
    pub todos: Vec<crate::db::todos::Todo>,
    /// Cursor into `todos`.
    pub selected: usize,
    /// Vertical scroll offset into this pane's list area.
    pub scroll_offset: usize,
}

impl TodoPane {
    pub fn selected_todo(&self) -> Option<&crate::db::todos::Todo> {
        self.todos.get(self.selected)
    }

    /// The list's free-form scratchpad note (stored in the legacy
    /// `carry_over` column), when it is not blank.
    pub fn scratchpad(&self) -> Option<&str> {
        self.list
            .as_ref()
            .and_then(|l| l.carry_over.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

/// State for the native TODOs overlay (`AppMode::Todos`): up to three scoped
/// panes, which one has focus, and any in-progress edit or prompt layered over
/// them.
pub struct TodoViewState {
    /// Project / feature indices the TODOs session lives under, used to
    /// resolve the session for selection on close and to host new lists.
    pub pi: usize,
    pub fi: usize,
    /// Display labels for the header.
    pub project_name: String,
    pub feature_name: String,
    /// The scoped panes, always ordered worktree → project → global. The
    /// worktree pane is absent for a feature on the repo root, which is the
    /// only way this is shorter than three.
    pub panes: Vec<TodoPane>,
    /// Index into `panes` of the visible pane that owns the cursor. `None` is
    /// valid for a repository-root feature when both optional scopes are
    /// hidden.
    pub focus: Option<usize>,
    /// Active inline edit, if any (add/edit title/notes/scratchpad).
    pub editor: Option<TodoEditor>,
    /// Whether inline edits open with the vim keymap. Remembered on the state
    /// (not the editor, which is rebuilt for each edit) for the life of the
    /// overlay; a fresh overlay starts with vim off, matching the other opt-in
    /// editor surfaces (compose). Toggled with `Ctrl+T`.
    pub todo_vim_enabled: bool,
    /// Set when a delete is awaiting y/n confirmation.
    pub pending_delete: bool,
    /// Active launch step (chooser / destination), layered over the list.
    pub launch: Option<TodoLaunchStep>,
    /// Active move/copy scope chooser (`M` / `C`), layered the same way.
    pub scope_move: Option<TodoScopeMoveState>,
}

impl TodoViewState {
    pub fn pane_is_visible(pane: &TodoPane, project_visible: bool, global_visible: bool) -> bool {
        match pane.kind {
            TodoPaneKind::Worktree => true,
            TodoPaneKind::Project => project_visible,
            TodoPaneKind::Global => global_visible,
        }
    }

    /// Indices of actionable panes in worktree → project → global order.
    pub fn visible_pane_indices(&self, project_visible: bool, global_visible: bool) -> Vec<usize> {
        self.panes
            .iter()
            .enumerate()
            .filter_map(|(index, pane)| {
                Self::pane_is_visible(pane, project_visible, global_visible).then_some(index)
            })
            .collect()
    }

    pub fn focused(&self) -> Option<&TodoPane> {
        self.focus.and_then(|focus| self.panes.get(focus))
    }

    pub fn focused_mut(&mut self) -> Option<&mut TodoPane> {
        self.focus.and_then(|focus| self.panes.get_mut(focus))
    }
}

/// The scope chooser raised by `M` (move) and `C` (copy) over the selected
/// TODO. Targets are pane indices rather than scopes so the in-memory pane and
/// the persisted list are updated from one lookup.
pub struct TodoScopeMoveState {
    /// `true` for a copy (leaves the original in place, unstarted), `false`
    /// for a move (re-files the same item, links and all).
    pub copy: bool,
    /// The item being re-filed, by id, so the list changing underneath the
    /// prompt is noticed rather than acted on stale.
    pub todo_id: String,
    pub todo_title: String,
    /// Candidate destinations as `(label, pane index)` — every pane but the
    /// one the item is already in.
    pub targets: Vec<(String, usize)>,
    pub selected: usize,
}

impl TodoScopeMoveState {
    pub fn move_cursor(&mut self, delta: isize) {
        if self.targets.is_empty() {
            return;
        }
        let last = self.targets.len() as isize - 1;
        self.selected = ((self.selected as isize) + delta).clamp(0, last) as usize;
    }
}

/// Single-line quick-capture of a TODO from inside a session view. The typed
/// title is appended to the session feature's own worktree list — falling back
/// to the project list when that feature sits on the repo root — auto-creating
/// the list (and a TODOs session under the current feature) when there is none
/// yet. `view` is the session view to return to on commit/cancel.
pub struct TodoQuickCaptureState {
    pub view: ViewState,
    /// Name of the project the TODO will be added to (shown in the dialog).
    pub project_name: String,
    /// Which list this capture will land in, named in the overlay so the
    /// target is never a guess (e.g. `"Worktree · add-login"`).
    pub list_label: String,
    /// The title being typed.
    pub input: String,
}

/// Prompt shown when the feature that hosts a project's TODO list is deleted
/// while the project survives (see `docs/backlog/feature-todos-plan.md`, Epic 1).
/// The user chooses which surviving feature re-homes the list, or deletes it.
pub struct TodosHostReassignState {
    /// Project whose list is being re-homed.
    pub project_name: String,
    /// Name of the just-deleted host feature (shown in the prompt).
    pub deleted_feature_name: String,
    /// `todo_lists.id` of the orphaned list.
    pub list_id: String,
    /// Surviving features the list can be re-homed onto: `(name, feature_id)`.
    pub candidates: Vec<(String, String)>,
    /// Selected option index: `0..candidates.len()` re-homes onto that feature;
    /// `== candidates.len()` deletes the list.
    pub selected: usize,
    /// Number of TODOs in the list (shown so the user knows what's at stake).
    pub todo_count: usize,
}

/// Which TODO an agent launch was started from, carried through the
/// create-feature wizard so the result can be linked back to the row it came
/// from.
///
/// The list id rides along with the todo id because the overlay may be closed
/// by the time the feature exists — a new-feature run leaves the Todos mode
/// entirely — so the link has to be written straight to the DB rather than
/// through the in-memory list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoPlanOrigin {
    pub todo_id: String,
    pub list_id: String,
    /// The TODO's title when the run started, captured rather than re-read:
    /// the title can be edited while the launch is in progress.
    pub todo_title: String,
    /// The feature hosting the TODO list when the run started.
    pub host_feature_id: String,
}

/// What `g`/`Enter` on an unlinked TODO offers: spawn in an existing feature
/// or spawn in a fresh worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoLaunchAction {
    /// Today's behavior: an agent session in the host feature (or, for a
    /// project/global TODO, a feature the user picks), composer seeded with the
    /// TODO, editable and unsent.
    SpawnSession,
    /// Create a new AIW feature and git worktree for this TODO, then seed
    /// its agent with the TODO.
    SpawnInNewFeature,
}

impl TodoLaunchAction {
    pub const ALL: [TodoLaunchAction; 2] = [
        TodoLaunchAction::SpawnSession,
        TodoLaunchAction::SpawnInNewFeature,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TodoLaunchAction::SpawnSession => "Start an agent on this TODO",
            TodoLaunchAction::SpawnInNewFeature => "Start an agent in a new feature",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            TodoLaunchAction::SpawnSession => {
                "Opens a session in this feature with the TODO in the composer, unsent."
            }
            TodoLaunchAction::SpawnInNewFeature => {
                "Creates a new branch and worktree, then seeds its agent with the TODO, unsent."
            }
        }
    }
}

/// The four answers to "this TODO already has work started for it".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoImplementChoice {
    /// Go to the feature or session the earlier run created.
    Jump,
    /// Start a second agent on it anyway, in the host feature.
    SpawnNew,
    /// Leave this one alone and scan on for the next candidate.
    SkipToNext,
    /// Change nothing and return to where the key was pressed.
    Cancel,
}

impl TodoImplementChoice {
    pub const ALL: [TodoImplementChoice; 4] = [
        TodoImplementChoice::Jump,
        TodoImplementChoice::SpawnNew,
        TodoImplementChoice::SkipToNext,
        TodoImplementChoice::Cancel,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TodoImplementChoice::Jump => "Go to the work already started",
            TodoImplementChoice::SpawnNew => "Start another agent on it",
            TodoImplementChoice::SkipToNext => "Skip it and take the next TODO",
            TodoImplementChoice::Cancel => "Cancel",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            TodoImplementChoice::Jump => {
                "Opens the feature or session this TODO was launched into."
            }
            TodoImplementChoice::SpawnNew => {
                "Opens a second session in the host feature, composer seeded and unsent."
            }
            TodoImplementChoice::SkipToNext => {
                "Leaves this one as it is and scans on for the next candidate."
            }
            TodoImplementChoice::Cancel => "Changes nothing.",
        }
    }
}

/// "Implement next" found that its best candidate already has work started for
/// it (`AppMode::TodoImplementChoice`).
///
/// Reached from two surfaces — a TODOs session row on the dashboard and the
/// TODOs overlay — so it is an [`AppMode`] of its own rather than a step
/// layered inside [`TodoViewState`], which only one of the two has. `origin`
/// is the mode the key was pressed in, restored verbatim on every exit: from
/// the overlay that returns the list with its cursor, scroll, and any unsaved
/// in-memory rows intact.
pub struct TodoImplementChoiceState {
    /// The mode to return to. Boxed because it may be a `Todos` overlay, whose
    /// state is large and (holding a `TextEditor`) not clonable.
    pub origin: Box<AppMode>,
    /// Project and fallback feature indices the scan ran under.
    pub pi: usize,
    pub fallback_fi: usize,
    /// The list's host feature id, when the list could be loaded.
    pub host_feature_id: Option<String>,
    /// Which scope the candidate came from. Decides whether *Start another
    /// agent on it* can spawn straight away (worktree) or has to ask which
    /// feature to spawn in (project / global).
    pub pane_kind: TodoPaneKind,
    /// The candidate: its id, so it is re-resolved by id alone on confirm
    /// (not trusted, and not tied to a remembered list — the item may have
    /// been moved to a different list while this prompt was open), and its
    /// title for display.
    pub todo_id: String,
    pub todo_title: String,
    /// TODOs already passed over by *Skip to next*, carried so a resumed scan
    /// does not offer the same item again. Ids, not indices: the list may be
    /// reordered underneath.
    pub skipped_ids: Vec<String>,
    pub selected: usize,
}

impl TodoImplementChoiceState {
    pub fn move_cursor(&mut self, delta: isize) {
        let last = TodoImplementChoice::ALL.len().saturating_sub(1) as isize;
        self.selected = ((self.selected as isize) + delta).clamp(0, last) as usize;
    }

    pub fn choice(&self) -> TodoImplementChoice {
        TodoImplementChoice::ALL[self.selected.min(TodoImplementChoice::ALL.len() - 1)]
    }
}

/// A feature to put an agent on a project- or global-scoped TODO in
/// (`AppMode::TodoSpawnTarget`).
///
/// A whole [`AppMode`] rather than a step inside [`TodoViewState`] for the
/// same reason [`TodoImplementChoiceState`] is one: the dashboard's "implement
/// next" reaches it with no overlay open. `origin` is the mode the key was
/// pressed in, restored verbatim on cancel, so the prompt never costs the user
/// their place in the list.
pub struct TodoSpawnTargetState {
    pub origin: Box<AppMode>,
    /// The item to spawn on, carried by value: the overlay it came from may
    /// not be open, and a global TODO's list is not reachable from `pi`.
    pub todo: crate::db::todos::Todo,
    /// Which scope the TODO came from, shown so the user knows why they are
    /// being asked.
    pub pane_kind: TodoPaneKind,
    /// Candidates as `(label, pi, fi)`. A project-scoped TODO lists that
    /// project's features; a global one lists every project's.
    pub candidates: Vec<(String, usize, usize)>,
    pub selected: usize,
    /// Set when the user explicitly asked for a second agent on an item that
    /// already has one, so the spawn does not reuse the existing session.
    pub force_new: bool,
}

impl TodoSpawnTargetState {
    pub fn move_cursor(&mut self, delta: isize) {
        if self.candidates.is_empty() {
            return;
        }
        let last = self.candidates.len() as isize - 1;
        self.selected = ((self.selected as isize) + delta).clamp(0, last) as usize;
    }

    pub fn selection(&self) -> Option<(usize, usize)> {
        self.candidates
            .get(self.selected)
            .map(|(_, pi, fi)| (*pi, *fi))
    }
}

/// What to do with the unfinished TODOs in a worktree list whose feature is
/// being deleted (`AppMode::TodoDeleteDisposition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoDeleteDisposition {
    MoveToProject,
    MoveToGlobal,
    Delete,
    Cancel,
}

impl TodoDeleteDisposition {
    pub const ALL: [TodoDeleteDisposition; 4] = [
        TodoDeleteDisposition::MoveToProject,
        TodoDeleteDisposition::MoveToGlobal,
        TodoDeleteDisposition::Delete,
        TodoDeleteDisposition::Cancel,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TodoDeleteDisposition::MoveToProject => "Move them to the project list",
            TodoDeleteDisposition::MoveToGlobal => "Move them to the global list",
            TodoDeleteDisposition::Delete => "Delete them with the worktree",
            TodoDeleteDisposition::Cancel => "Cancel — keep the feature",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            TodoDeleteDisposition::MoveToProject => {
                "They stay with the project and show up in its pane."
            }
            TodoDeleteDisposition::MoveToGlobal => {
                "They leave the project for the machine-wide list."
            }
            TodoDeleteDisposition::Delete => "The items go away for good, with the list.",
            TodoDeleteDisposition::Cancel => {
                "Nothing is deleted — the feature and its worktree stay."
            }
        }
    }
}

/// The prompt raised before a feature is deleted while its worktree list still
/// has unfinished items. Deleting a worktree is hard to reverse, so this is a
/// blocking prompt: nothing is killed or removed until a choice is made, and
/// *Cancel* returns to the dashboard with the feature intact.
pub struct TodoDeleteDispositionState {
    pub project_name: String,
    pub feature_name: String,
    /// The doomed feature's id. Carried so *Move to the project list* can host
    /// a newly-created project list on a feature that will still be there
    /// afterwards — hosting it on this one would hand the items straight to
    /// the orphaned-list cleanup that runs once the deletion completes.
    pub feature_id: String,
    /// The worktree list and where it lives, so the disposition can be applied
    /// without re-deriving it from indices that the deletion will invalidate.
    pub project_id: String,
    pub workdir: String,
    pub list_id: String,
    /// How many items are still open, stated in the prompt.
    pub unfinished: usize,
    pub selected: usize,
}

impl TodoDeleteDispositionState {
    pub fn move_cursor(&mut self, delta: isize) {
        let last = TodoDeleteDisposition::ALL.len() as isize - 1;
        self.selected = ((self.selected as isize) + delta).clamp(0, last) as usize;
    }

    pub fn choice(&self) -> TodoDeleteDisposition {
        TodoDeleteDisposition::ALL[self.selected.min(TodoDeleteDisposition::ALL.len() - 1)]
    }
}

/// A modal step layered over the TODO list, the way `pending_delete` and
/// `editor` are: the list, cursor, and scroll stay intact underneath, so `Esc`
/// returns to exactly what the user was looking at.
///
/// A separate [`AppMode`] would replace [`TodoViewState`] wholesale and force
/// the list to be reloaded — and the in-memory list is the overlay's source of
/// truth, so it would also lose unsaved state on a DB-less run.
#[derive(Debug, Clone)]
pub enum TodoLaunchStep {
    /// `g`/`Enter` on a TODO with no existing link: pick where to spawn.
    Choice {
        origin: TodoPlanOrigin,
        selected: usize,
    },
}

impl TodoLaunchStep {
    pub fn origin(&self) -> &TodoPlanOrigin {
        match self {
            TodoLaunchStep::Choice { origin, .. } => origin,
        }
    }

    pub fn selected(&self) -> usize {
        match self {
            TodoLaunchStep::Choice { selected, .. } => *selected,
        }
    }

    pub fn option_count(&self) -> usize {
        match self {
            TodoLaunchStep::Choice { .. } => TodoLaunchAction::ALL.len(),
        }
    }

    /// Move the cursor, clamped rather than wrapped: with two options, wrapping
    /// makes `j` and `k` the same key and the highlight appears not to move.
    pub fn move_cursor(&mut self, delta: isize) {
        let last = self.option_count().saturating_sub(1);
        let current = self.selected() as isize;
        let next = (current + delta).clamp(0, last as isize) as usize;
        match self {
            TodoLaunchStep::Choice { selected, .. } => *selected = next,
        }
    }

    /// The chosen action.
    pub fn action(&self) -> Option<TodoLaunchAction> {
        match self {
            TodoLaunchStep::Choice { selected, .. } => {
                Some(TodoLaunchAction::ALL[(*selected).min(TodoLaunchAction::ALL.len() - 1)])
            }
        }
    }
}

pub enum AppMode {
    Normal,
    Todos(TodoViewState),
    TodoQuickCapture(TodoQuickCaptureState),
    /// Re-home or delete a project's TODO list after its host feature is deleted.
    TodosHostReassign(TodosHostReassignState),
    /// "Implement next" landed on a TODO that already has work started for it.
    TodoImplementChoice(Box<TodoImplementChoiceState>),
    /// Pick the feature to put an agent on a project- or global-scoped TODO
    /// in. Those lists are not tied to one checkout, so there is no feature to
    /// infer — the user names one, and it supplies the agent and mode.
    TodoSpawnTarget(Box<TodoSpawnTargetState>),
    /// A feature is about to be deleted and its worktree list still has
    /// unfinished TODOs: move them to the project list, move them to the
    /// global list, delete them, or cancel the deletion outright.
    TodoDeleteDisposition(TodoDeleteDispositionState),
    /// Confirm completing the TODO explicitly referenced by the embedded
    /// agent session currently being viewed.
    ConfirmTodoReferenceCompletion(TodoReferenceCompletionState),
    CreatingProject(CreateProjectState),
    CreatingFeature(CreateFeatureState),
    DeletingProject(String),
    DeletingFeature(String, String),
    DeletingFeatureInProgress(DeletingFeatureState),
    Help(HelpState),
    NotificationPicker(usize, Option<ViewState>),
    RenamingSession(RenameSessionState),
    RenamingFeature(RenameFeatureState),
    SessionConfig(SessionConfigState),
    ProjectAgentConfig(ProjectAgentConfigState),
    BrowsingPath(Box<BrowsePathState>),
    CommandPicker(super::CommandPickerState),
    Searching(SearchState),
    NamingNewSession(NewSessionNameState),
    ClaudeSessionPicker(ClaudeSessionPickerState),
    ConfirmingClaudeSession {
        session_id: String,
        workdir: PathBuf,
        target_session_id: Option<String>,
    },
    CodexSessionPicker(CodexSessionPickerState),
    ConfirmingCodexSession {
        session_id: String,
        workdir: PathBuf,
        target_session_id: Option<String>,
    },
    StoppedSessionDialog(StoppedSessionDialogState),
    BookmarkPicker(BookmarkPickerState),
    DiffPicker(DiffPickerState),
    DiffViewerLoading(DiffViewerState),
    DiffViewer(DiffViewerState),
    /// Browse open issues for the selected project's canonical GitHub repo.
    IssueBrowser(crate::app::issue_fixer::IssueBrowserState),
    /// Configure the feature and editable prompt for the selected issue.
    IssueSetup(crate::app::issue_fixer::IssueSetupState),
    /// An existing feature already records the same canonical issue. The
    /// carried setup preserves all edits if the user cancels the override.
    IssueDuplicateWarning(crate::app::issue_fixer::IssueDuplicateWarningState),
    SessionPicker(SessionPickerState),
    DiffReviewPrompt(DiffReviewState),
    RunningHook(RunningHookState),
    HookPrompt(HookPromptState),
    // FORK: one-time approval of a repository's lifecycle hook.
    ApprovingRepoHook(Box<crate::fork::hook_approval::HookApprovalState>),
    // FORK: a pending permission request's tool and full input (P6).
    PermissionDetail(Box<super::fork_permissions::PermissionDetailState>),
    // FORK: the "Queue a follow-up" dialog (P7).
    FollowupQueue(Box<super::fork_followups::FollowupQueueState>),
    LatestPrompt(LatestPromptState),
    PromptLibrary(PromptLibraryState),
    PromptEditor(PromptEditorState),
    PlaceholderFill(PlaceholderFillState),
    SkillPicker(SkillPickerState),
    ForkingFeature(ForkFeatureState),
    /// FORK: `f` — where a fork of a session's conversation goes (§19.2).
    ForkSession(crate::app::fork_identity::ForkSessionState),
    /// FORK: `E` — Claude sessions outside AIW in the projects (§19.2).
    ExternalSessions(crate::app::fork_identity::ExternalSessionsState),
    ThemePicker(ThemePickerState),
    SyntaxLanguagePicker(SyntaxLanguagePickerState),
    DebugLog(DebugLogState),
    MarkdownLoading(MarkdownLoadingState),
    MarkdownViewer(MarkdownViewerState),
    MarkdownFilePicker(MarkdownFilePickerState),
    CreatingBatchFeatures(CreateBatchFeaturesState),
    HarnessSetup(HarnessSetupState),
    // Boxed: the largest mode by a wide margin (`clippy::large_enum_variant`).
    ConfigWizard(Box<ConfigWizardState>),
    /// Choosing which harness runs a fresh dedicated session for a finished
    /// review's fixes (only shown when the dedicated fix target is selected and
    /// no review session exists yet). The feedback file is already written; this
    /// only governs where the "address the feedback" prompt is dispatched.
    /// Soft warning shown before starting an agent when the machine is already
    /// at the concurrency cap and/or low on memory. Confirming starts anyway.
    ConfirmResourceStart(Box<ResourceConfirmState>),
    /// Features that are idle and unattended, with per-row reclaim actions.
    Dormant(DormantViewState),
    /// Global context-window/severity settings (`w` on the dashboard).
    ContextSettings(ContextSettingsState),
    // FORK(no-cloud): the Remote Control `RemotePairing` mode was removed.
}

/// The view to return to plus the stable TODO identity to complete.
pub struct TodoReferenceCompletionState {
    pub view: ViewState,
    pub todo_id: String,
}

#[derive(Debug, Clone)]
pub struct HarnessOption {
    pub kind: AgentKind,
    pub status: HarnessCheckStatus,
    pub enabled: bool,
    /// FORK: the user pressed Enter while the opening sweep was still
    /// probing this row; tick it when that probe finds the CLI, instead of
    /// probing a second time.
    pub tick_when_found: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HarnessCheckStatus {
    Unchecked,
    Checking,
    Installed,
    NotFound(String),
}

#[derive(Debug, Clone)]
pub struct HarnessSetupState {
    pub selected: usize,
    pub harnesses: Vec<HarnessOption>,
    pub is_startup: bool,
}

impl HarnessSetupState {
    pub fn new(is_startup: bool, existing: &[AgentKind]) -> Self {
        let harnesses = AgentKind::ALL
            .iter()
            .map(|kind| {
                let already_enabled = existing.contains(kind);
                HarnessOption {
                    kind: kind.clone(),
                    status: if already_enabled {
                        HarnessCheckStatus::Installed
                    } else {
                        HarnessCheckStatus::Unchecked
                    },
                    enabled: already_enabled,
                    tick_when_found: false,
                }
            })
            .collect();
        Self {
            selected: 0,
            harnesses,
            is_startup,
        }
    }

    pub fn enabled_harnesses(&self) -> Vec<AgentKind> {
        self.harnesses
            .iter()
            .filter(|h| h.enabled)
            .map(|h| h.kind.clone())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigCategory {
    CustomSessions,
    FeaturePresets,
    LifecycleHooks,
    Keybindings,
    AllowedAgents,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigScope {
    Global,
    Project(PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigWizardStep {
    CategoryPicker,
    ScopePicker,
    ItemList,
    EditItem,
    ConfirmSave,
}

pub struct ConfigWizardState {
    pub step: ConfigWizardStep,
    pub category: ConfigCategory,
    pub scope: ConfigScope,
    pub selected: usize,
    pub field_focus: usize,
    pub input_mode: bool,
    pub sessions: Vec<CustomSessionConfig>,
    pub presets: Vec<FeaturePreset>,
    pub hooks: LifecycleHooks,
    pub keybindings: HashMap<String, char>,
    pub allowed_agents: Option<Vec<AgentKind>>,
    pub editing_index: Option<usize>,
    pub field_values: Vec<String>,
    pub field_editor: Option<ConfigWizardFieldEditor>,
    pub icon_picker: Option<ConfigWizardIconPicker>,
    pub field_toggles: Vec<bool>,
    pub agent_toggles: Vec<bool>,
    pub agent_toggles_dirty: bool,
    pub keybinding_actions: Vec<String>,
    pub capturing_key: bool,
    pub original_json: String,
    pub modified_json: String,
    pub confirm_diff: Option<crate::diff::DiffFile>,
    pub preview_scroll: usize,
    pub project_repo: Option<PathBuf>,
    pub project_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigWizardIconPicker {
    pub selected: usize,
}

pub struct ConfigWizardFieldEditor {
    pub field_index: usize,
    pub label: String,
    pub editor: TextEditor,
    pub scroll_offset: usize,
    pub sync_scroll_to_cursor: bool,
}

pub enum ThemePickerEntry {
    Theme(crate::theme::ThemeName),
    Group {
        label: &'static str,
        themes: Vec<crate::theme::ThemeName>,
    },
    /// FORK: non-selectable section title ("Dark" / "Light").
    Header(&'static str),
}

impl ThemePickerEntry {
    /// The theme this entry previews when highlighted: itself for a single
    /// theme, or the first member for a group (a representative peek).
    pub fn preview_theme(&self) -> Option<crate::theme::ThemeName> {
        match self {
            ThemePickerEntry::Theme(name) => Some(*name),
            ThemePickerEntry::Group { themes, .. } => themes.first().copied(),
            ThemePickerEntry::Header(_) => None, // FORK
        }
    }

    /// The full set of top-level entries, with subtype-heavy families
    /// (Catppuccin, Gruvbox Material) collapsed into groups so they don't
    /// clog the single-level list.
    pub fn build() -> Vec<Self> {
        crate::fork::theme_picker::entries() // FORK: grouped into dark / light sections
    }
}

/// Second-screen state for a group entry drilled into from the top-level
/// list; `None` means the picker is showing the top-level list.
pub struct ThemePickerGroupState {
    pub label: &'static str,
    pub themes: Vec<crate::theme::ThemeName>,
    pub selected: usize,
}

pub struct ThemePickerState {
    pub selected: usize,
    pub entries: Vec<ThemePickerEntry>,
    pub original_theme: crate::theme::ThemeName,
    /// The theme currently rendered on screen (live preview). Drilling into
    /// a group lands on this theme when it's a member, so where `Enter` puts
    /// the cursor always matches what the user is already looking at.
    pub previewed: crate::theme::ThemeName,
    pub group: Option<ThemePickerGroupState>,
}

pub struct SyntaxLanguageRow {
    pub language: crate::highlight::HighlightLanguage,
    pub status: crate::highlight::HighlightInstallState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxOperationAction {
    Install,
    Uninstall,
}

pub enum SyntaxOperationEvent {
    Output(String),
    Finished(Result<String, String>),
}

pub struct SyntaxOperationState {
    pub language: crate::highlight::HighlightLanguage,
    pub action: SyntaxOperationAction,
    pub last_output: Option<String>,
    pub started_at: std::time::Instant,
    pub output_rx: std::sync::mpsc::Receiver<SyntaxOperationEvent>,
}

pub struct SyntaxLanguagePickerState {
    pub languages: Vec<SyntaxLanguageRow>,
    pub selected: usize,
    pub notice: Option<String>,
    pub operation: Option<SyntaxOperationState>,
    pub return_to: Option<Box<AppMode>>,
    pub auto_return_on_success: bool,
    pub return_language: Option<crate::highlight::HighlightLanguage>,
}

pub struct DebugLogState {
    pub scroll_offset: usize,
    pub from_view: Option<ViewState>,
    pub hide_perf_logs: bool,
}

pub struct MarkdownViewerState {
    pub title: String,
    pub source_path: PathBuf,
    pub content: String,
    pub scroll_offset: usize,
    pub rendered_width: u16,
    pub rendered_lines: Vec<ratatui::text::Line<'static>>,
    pub return_to_picker: Option<MarkdownFilePickerState>,
    pub from_view: Option<ViewState>,
}

pub enum MarkdownLoadingOperation {
    DiscoverFromView {
        view: ViewState,
    },
    DiscoverFromViewer {
        viewer: MarkdownViewerState,
    },
    ReadPath {
        path: PathBuf,
        workdir: PathBuf,
        repo_root: Option<PathBuf>,
        view: ViewState,
        return_to_picker: Option<MarkdownFilePickerState>,
    },
}

pub struct MarkdownLoadingState {
    pub title: String,
    pub from_view: Option<ViewState>,
    pub operation: MarkdownLoadingOperation,
}

pub struct MarkdownFilePickerState {
    pub files: Vec<PathBuf>,
    pub selected: usize,
    pub plan_only: bool,
    pub search_active: bool,
    pub query: String,
    pub workdir: PathBuf,
    pub repo_root: Option<PathBuf>,
    pub from_view: Option<ViewState>,
}

#[derive(Clone)]
pub struct SessionPickerState {
    pub builtin_sessions: Vec<BuiltinSessionOption>,
    pub custom_sessions: Vec<CustomSessionConfig>,
    pub selected: usize,
    pub pi: usize,
    pub fi: usize,
    pub from_view: Option<ViewState>,
}

#[derive(Clone)]
pub struct BuiltinSessionOption {
    pub kind: crate::project::SessionKind,
    pub label: String,
    pub disabled: Option<String>,
}

/// The dormant-features overlay: features that are idle *and* unattended, with
/// what each is still holding.
pub struct DormantViewState {
    pub features: Vec<crate::app::dormant::DormantFeature>,
    pub selected: usize,
    /// Result of the last action, shown in the overlay's footer.
    pub message: Option<String>,
}

impl DormantViewState {
    pub fn selected_feature(&self) -> Option<&crate::app::dormant::DormantFeature> {
        self.features.get(self.selected)
    }

    /// Keep the cursor on a real row after the list shrinks.
    pub fn clamp_selection(&mut self) {
        if self.selected >= self.features.len() {
            self.selected = self.features.len().saturating_sub(1);
        }
    }
}

/// A harness start paused on the resource-gate confirmation, replayed verbatim
/// if the user confirms and dropped if they cancel.
#[derive(Debug, Clone, PartialEq)]
pub enum PendingStart {
    /// Starting a stopped feature (`c` on the dashboard).
    Feature { pi: usize, fi: usize },
    /// Adding a session to a feature (session picker) that will spawn a
    /// harness — either the session itself, or the stopped feature's saved
    /// agents coming up underneath it.
    BuiltinSession {
        pi: usize,
        fi: usize,
        kind: SessionKind,
        label: Option<String>,
    },
    /// Recreating one session's tmux window (`c` on a session whose window
    /// was stopped individually with `x`) while the rest of its feature
    /// stays up.
    RestartSessionWindow { pi: usize, fi: usize, si: usize },
}

/// The pre-start warning: what tripped, what it was about to do, and where to
/// go back to afterwards.
pub struct ResourceConfirmState {
    pub over_limit: Option<crate::app::resource_gate::OverLimit>,
    pub low_memory: Option<crate::app::resource_gate::LowMemory>,
    /// Editor windows open right now, collected only when the memory half
    /// tripped: they are not agents and are not counted as such, but they are
    /// usually the larger half of where the memory went.
    pub open_editors: Vec<String>,
    pub pending: PendingStart,
    /// Session view to restore after confirming or cancelling, when the start
    /// was initiated from inside an embedded session rather than the dashboard.
    pub from_view: Option<ViewState>,
}

// Hook continuations exist one at a time and are moved directly into the
// worker. Keeping the launch fields flat makes the recovery path explicit;
// boxing selected fields solely to equalize enum variants would obscure it.
#[allow(clippy::large_enum_variant)]
pub enum HookNext {
    WorktreeCreated {
        project_name: String,
        feature_name: Option<String>,
        branch: String,
        mode: VibeMode,
        agent: AgentKind,
        create_terminal: bool,
        session_name: String,
        enable_chrome: bool,
        startup_prompt: Option<String>,
        /// Carried across the hook detour, which rebuilds the launch from
        /// scratch and would otherwise drop the TODO link on any project with
        /// an `on_worktree_created` hook.
        todo_origin: Option<TodoPlanOrigin>,
        issue_source: Option<crate::project::IssueSource>,
    },
    StartFeature {
        pi: usize,
        fi: usize,
        /// The session row the start was requested from, launched on purpose.
        session_id: Option<String>,
    },
    StopFeature {
        pi: usize,
        fi: usize,
    },
}

pub struct HookPromptState {
    pub script: String,
    pub workdir: PathBuf,
    pub title: String,
    pub options: Vec<String>,
    pub selected: usize,
    pub next: HookNext,
}

pub struct RunningHookState {
    pub script: String,
    pub workdir: PathBuf,
    pub project_name: String,
    pub feature_name: Option<String>,
    pub todo_origin: Option<TodoPlanOrigin>,
    /// Source issue supplied by the issue-fixer workflow.
    pub issue_source: Option<crate::project::IssueSource>,
    pub branch: String,
    pub mode: VibeMode,
    pub agent: AgentKind,
    pub create_terminal: bool,
    pub session_name: String,
    pub enable_chrome: bool,
    pub startup_prompt: Option<String>,
    pub child: Option<Child>,
    pub output: String,
    pub success: Option<bool>,
    pub output_rx: Option<std::sync::mpsc::Receiver<String>>,
    /// FORK: when the hook was started, for the "still running" notice.
    pub started_at: Instant,
    /// FORK: the notice after [`RunningHookState::TIMEOUT`] was appended.
    pub timeout_noted: bool,
    /// FORK: the user skipped the hook (killed) after it overran.
    pub skipped: bool,
}

impl RunningHookState {
    /// FORK: after this long a hanging hook can be skipped with Esc.
    pub const TIMEOUT: Duration = Duration::from_secs(60);

    pub fn key(&self) -> String {
        format!("{}/{}", self.workdir.display(), self.script)
    }

    /// FORK: the hook has overrun [`Self::TIMEOUT`] and is still running.
    pub fn overran(&self) -> bool {
        self.child.is_some() && self.started_at.elapsed() >= Self::TIMEOUT
    }
}

pub struct DeletingFeatureState {
    pub project_name: String,
    pub feature_name: String,
    pub tmux_session: String,
    pub is_worktree: bool,
    pub repo: PathBuf,
    pub workdir: PathBuf,
    pub stage: DeleteStage,
    pub child: Option<Child>,
    pub output: String,
    pub output_rx: Option<std::sync::mpsc::Receiver<String>>,
    pub error: Option<String>,
}

impl DeletingFeatureState {
    pub fn key(&self) -> String {
        format!("{}/{}", self.project_name, self.feature_name)
    }
}

pub struct BackgroundDeletion {
    pub project_name: String,
    pub feature_name: String,
    pub tmux_session: String,
    pub is_worktree: bool,
    pub repo: PathBuf,
    pub workdir: PathBuf,
    pub stage: DeleteStage,
    pub child: Option<Child>,
    pub output: String,
    pub output_rx: Option<std::sync::mpsc::Receiver<String>>,
    pub error: Option<String>,
}

impl BackgroundDeletion {
    pub fn from_deleting_state(state: DeletingFeatureState) -> Self {
        Self {
            project_name: state.project_name,
            feature_name: state.feature_name,
            tmux_session: state.tmux_session,
            is_worktree: state.is_worktree,
            repo: state.repo,
            workdir: state.workdir,
            stage: state.stage,
            child: state.child,
            output: state.output,
            output_rx: state.output_rx,
            error: state.error,
        }
    }
}

pub struct BackgroundHook {
    #[allow(dead_code)] // retained for the background-hook key, not read directly
    pub script: String,
    pub workdir: PathBuf,
    pub project_name: String,
    pub feature_name: Option<String>,
    pub todo_origin: Option<TodoPlanOrigin>,
    pub issue_source: Option<crate::project::IssueSource>,
    pub branch: String,
    pub mode: VibeMode,
    pub agent: AgentKind,
    pub create_terminal: bool,
    pub session_name: String,
    pub enable_chrome: bool,
    pub startup_prompt: Option<String>,
    pub child: Option<Child>,
    pub output: String,
    pub success: Option<bool>,
    pub output_rx: Option<std::sync::mpsc::Receiver<String>>,
}

impl BackgroundHook {
    pub fn from_running_state(state: RunningHookState) -> Self {
        Self {
            script: state.script,
            workdir: state.workdir,
            project_name: state.project_name,
            feature_name: state.feature_name,
            todo_origin: state.todo_origin,
            issue_source: state.issue_source,
            branch: state.branch,
            mode: state.mode,
            agent: state.agent,
            create_terminal: state.create_terminal,
            session_name: state.session_name,
            enable_chrome: state.enable_chrome,
            startup_prompt: state.startup_prompt,
            child: state.child,
            output: state.output,
            success: state.success,
            output_rx: state.output_rx,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum DeleteStage {
    KillingTmux,
    RemovingWorktree,
    Completed,
}

pub struct BrowsePathState {
    pub explorer: FileExplorer,
    pub create_state: CreateProjectState,
    pub new_folder_name: String,
    pub creating_folder: bool,
}

#[derive(Clone)]
pub struct CreateProjectState {
    pub step: CreateProjectStep,
    pub name: String,
    pub path: String,
    pub agent: AgentKind,
    pub agent_index: usize,
}

#[derive(Clone, PartialEq)]
pub enum CreateProjectStep {
    Name,
    Path,
    Agent,
}

impl CreateProjectState {
    pub fn auto_detect() -> Self {
        let cwd = std::env::current_dir().unwrap_or_default();
        let repo_path = crate::worktree::WorktreeManager::repo_root(&cwd)
            .unwrap_or(cwd)
            .to_string_lossy()
            .into_owned();
        Self {
            step: CreateProjectStep::Name,
            name: String::new(),
            path: repo_path,
            agent: AgentKind::default(),
            agent_index: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CreateFeatureStep {
    Source,
    ExistingWorktree,
    SelectPreset,
    Branch,
    Worktree,
    Mode,
    /// FORK: §19.3 — shared worktree or one each (only with several agents).
    FanOut,
    SessionName,
    /// FORK: §19.3 — the task prompt every agent receives, unsent unless
    /// "send when ready" is ticked.
    Prompt,
    ConfirmSuperVibe,
}

/// FORK: §19.3 — the prompt library inside the Prompt step (`Ctrl+L`).
#[derive(Debug, Clone, Default)]
pub struct FeaturePromptPicker {
    /// `(name, body)` of every template the library would show.
    pub entries: Vec<(String, String)>,
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CreateBatchFeaturesStep {
    WorkspacePath,
    ProjectName,
    FeatureCount,
    FeatureBaseName,
    FeatureSettings,
}

pub struct CreateFeatureState {
    pub project_name: String,
    pub project_repo: PathBuf,
    /// Set when the wizard was opened from a TODO, so the feature it creates
    /// can be linked back to that row. Lives on the wizard state (not on
    /// `App`) so cancelling the wizard drops it — a stale origin would attach
    /// the next unrelated feature to the wrong TODO.
    pub todo_origin: Option<TodoPlanOrigin>,
    pub issue_source: Option<crate::project::IssueSource>,
    /// Optional display name supplied by a workflow with separate feature and
    /// branch fields. Ordinary feature creation keeps this unset.
    pub feature_name: Option<String>,
    pub branch: String,
    pub branch_error: Option<String>,
    pub allowed_agents: Vec<AgentKind>,
    pub feature_presets: Vec<FeaturePreset>,
    pub step: CreateFeatureStep,
    pub agent: AgentKind,
    pub agent_index: usize,
    pub mode: VibeMode,
    pub mode_index: usize,
    pub mode_focus: usize,
    pub create_terminal: bool,
    pub session_name: String,
    pub source_index: usize,
    pub worktrees: Vec<WorktreeInfo>,
    pub worktree_index: usize,
    pub worktree_search_active: bool,
    pub worktree_query: String,
    pub use_worktree: bool,
    pub enable_chrome: bool,
    pub preset_index: usize,
    pub task_prompt: String,
    /// FORK: §19.3 — harnesses ticked with Space, in tick order; the first is
    /// the primary. Empty means the single highlighted harness (`agent`).
    pub ticked: Vec<AgentKind>,
    /// FORK: §19.3 — how several agents are laid out.
    pub fan_out: crate::fork::fan_out::FanOutLayout,
    /// FORK: §19.3 — press Enter for every agent once its pane is ready.
    pub send_prompt: bool,
    /// FORK: §19.3 — the open `Ctrl+L` picker in the Prompt step.
    pub prompt_picker: Option<FeaturePromptPicker>,
    /// FORK: §19.3 — why the mode changed (Codex in a Vibeless set).
    pub mode_note: Option<String>,
}

impl CreateFeatureState {
    pub fn new(
        project_name: String,
        project_repo: PathBuf,
        worktrees: Vec<WorktreeInfo>,
        is_first_feature: bool,
    ) -> Self {
        // FORK: never pre-fill the branch with the currently checked-out one
        // (AIW used the process cwd's branch): that branch is already in use,
        // so a worktree create on it can only fail.
        let branch = String::new();

        let step = if worktrees.is_empty() {
            CreateFeatureStep::Branch
        } else {
            CreateFeatureStep::Source
        };
        Self {
            project_name,
            project_repo,
            todo_origin: None,
            issue_source: None,
            feature_name: None,
            branch,
            branch_error: None,
            allowed_agents: AgentKind::ALL.to_vec(),
            feature_presets: Vec::new(),
            step,
            agent: AgentKind::default(),
            agent_index: 0,
            mode: VibeMode::default(),
            mode_index: 0,
            mode_focus: 0,
            create_terminal: false,
            session_name: "Claude 1".to_string(),
            source_index: 0,
            worktrees,
            worktree_index: 0,
            worktree_search_active: false,
            worktree_query: String::new(),
            use_worktree: !is_first_feature,
            enable_chrome: false,
            preset_index: 0,
            task_prompt: String::new(),
            ticked: Vec::new(),
            fan_out: crate::fork::fan_out::FanOutLayout::default(),
            send_prompt: false,
            prompt_picker: None,
            mode_note: None,
        }
    }

    /// FORK: §19.3 — the harnesses this feature starts, primary first.
    pub fn selected_agents(&self) -> Vec<AgentKind> {
        if self.ticked.is_empty() {
            vec![self.agent.clone()]
        } else {
            self.ticked.clone()
        }
    }

    /// FORK: §19.3 — more than one agent is ticked.
    pub fn is_fan_out(&self) -> bool {
        self.ticked.len() > 1
    }

    /// FORK: §19.3 — sibling worktrees need a new branch in a new worktree;
    /// the repo dir and an existing worktree can only be shared.
    pub fn worktree_each_possible(&self) -> bool {
        let existing = self.source_index == 1 && !self.worktrees.is_empty();
        !existing && self.use_worktree
    }

    /// FORK: §19.3 — the layout the launch will use.
    pub fn effective_fan_out(&self) -> crate::fork::fan_out::FanOutLayout {
        if self.worktree_each_possible() {
            self.fan_out
        } else {
            crate::fork::fan_out::FanOutLayout::Shared
        }
    }

    /// FORK: §19.3 — Space on the highlighted harness.
    pub fn toggle_tick(&mut self) {
        let Some(agent) = self.allowed_agents.get(self.agent_index).cloned() else {
            return;
        };
        if let Some(pos) = self.ticked.iter().position(|a| *a == agent) {
            self.ticked.remove(pos);
        } else {
            self.ticked.push(agent);
        }
        self.sync_primary();
    }

    /// FORK: §19.3 — `agent` follows the first ticked harness, else the
    /// highlighted one.
    pub fn sync_primary(&mut self) {
        if let Some(first) = self.ticked.first() {
            self.agent = first.clone();
        } else if let Some(agent) = self.allowed_agents.get(self.agent_index) {
            self.agent = agent.clone();
        }
    }

    /// FORK: §19.3 — where the Mode step goes next (after SuperVibe is
    /// confirmed, if it was picked): the layout choice, the prompt, or the
    /// session name.
    pub fn advance_past_mode(&mut self) {
        if self.is_fan_out() {
            let (mode, note) =
                crate::fork::fan_out::fan_out_mode(&self.selected_agents(), &self.mode);
            if let Some(index) = VibeMode::ALL.iter().position(|m| *m == mode) {
                self.mode_index = index;
            }
            self.mode = mode;
            self.mode_note = note;
            self.step = if self.worktree_each_possible() {
                CreateFeatureStep::FanOut
            } else {
                CreateFeatureStep::Prompt
            };
        } else {
            self.mode_note = None;
            self.session_name = crate::app::App::default_session_name_for_agent(&self.agent);
            self.step = CreateFeatureStep::SessionName;
        }
    }

    /// FORK: §19.3 — the step `Esc` returns to from the Prompt step.
    pub fn step_before_prompt(&self) -> CreateFeatureStep {
        if !self.is_fan_out() {
            CreateFeatureStep::SessionName
        } else if self.worktree_each_possible() {
            CreateFeatureStep::FanOut
        } else {
            CreateFeatureStep::Mode
        }
    }

    pub fn focused_mode_description(&self) -> Option<&'static str> {
        match self.mode_focus {
            0 | 1 => None,
            2 if self.agent == AgentKind::Claude => {
                Some("Enable browser automation for features that need Chrome.")
            }
            _ => None,
        }
    }

    pub fn visible_worktree_indices(&self) -> Vec<usize> {
        let mut matches: Vec<(usize, usize)> = self
            .worktrees
            .iter()
            .enumerate()
            .filter_map(|(idx, worktree)| {
                let score =
                    crate::app::util::worktree_picker_score(worktree, &self.worktree_query)?;
                Some((idx, score))
            })
            .collect();

        matches.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        matches.into_iter().map(|(idx, _)| idx).collect()
    }

    pub fn clamp_worktree_selection(&mut self) {
        let visible = self.visible_worktree_indices();
        if visible.is_empty() {
            self.worktree_index = 0;
        } else if !visible.contains(&self.worktree_index) {
            self.worktree_index = visible[0];
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedFeatureLaunch {
    pub project_name: String,
    /// Separate display/persisted feature name for workflows whose branch is
    /// user-editable. Ordinary creation leaves this `None` and uses `branch`.
    pub feature_name: Option<String>,
    pub branch: String,
    pub workdir: PathBuf,
    pub is_worktree: bool,
    pub mode: VibeMode,
    pub agent: AgentKind,
    pub create_terminal: bool,
    pub session_name: String,
    pub enable_chrome: bool,
    pub hook_succeeded: Option<bool>,
    /// Optional composer seed to show immediately after the agent starts.
    pub startup_prompt: Option<String>,
    /// Set when this launch was started from a TODO, so accepting the plan can
    /// link the created feature back to the row it came from.
    pub todo_origin: Option<TodoPlanOrigin>,
    pub issue_source: Option<crate::project::IssueSource>,
}

#[derive(Clone)]
pub struct CreateBatchFeaturesState {
    pub workspace_path: String,
    pub project_name: String,
    pub feature_count: usize,
    pub feature_prefix: String,
    pub agent: AgentKind,
    pub agent_index: usize,
    pub mode: VibeMode,
    pub mode_index: usize,
    pub mode_focus: usize,
    pub enable_chrome: bool,
    pub step: CreateBatchFeaturesStep,
}

impl CreateBatchFeaturesState {
    pub fn with_workspace(workspace_path: Option<String>) -> Self {
        let repo_path = if let Some(ws) = workspace_path {
            ws
        } else {
            let cwd = std::env::current_dir().unwrap_or_default();
            crate::worktree::WorktreeManager::repo_root(&cwd)
                .unwrap_or(cwd)
                .to_string_lossy()
                .into_owned()
        };
        let workspace_name = std::path::Path::new(&repo_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace")
            .to_string();

        Self {
            workspace_path: repo_path,
            project_name: workspace_name,
            feature_count: 3,
            feature_prefix: "feature".to_string(),
            agent: AgentKind::default(),
            agent_index: 0,
            mode: VibeMode::default(),
            mode_index: 0,
            mode_focus: 0,
            enable_chrome: false,
            step: CreateBatchFeaturesStep::WorkspacePath,
        }
    }
}

pub const DASHBOARD_SESSION_FILTER_ENABLED: bool = false;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum SessionFilter {
    #[default]
    All,
    Claude,
    Codex,
    Terminal,
    Nvim,
    Vscode,
}

impl SessionFilter {
    pub const ALL: [SessionFilter; 6] = [
        SessionFilter::All,
        SessionFilter::Claude,
        SessionFilter::Codex,
        SessionFilter::Terminal,
        SessionFilter::Nvim,
        SessionFilter::Vscode,
    ];

    pub fn display_name(&self) -> &str {
        match self {
            SessionFilter::All => "all",
            SessionFilter::Claude => "claude",
            SessionFilter::Codex => "codex",
            SessionFilter::Terminal => "terminal",
            SessionFilter::Nvim => "nvim",
            SessionFilter::Vscode => "vscode",
        }
    }

    pub fn next(&self) -> Self {
        let variants = Self::ALL.as_slice();
        let idx = variants.iter().position(|v| v == self).unwrap_or(0);
        variants[(idx + 1) % variants.len()].clone()
    }
}

pub struct SearchState {
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub selected_match: usize,
}

#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub item: VisibleItem,
    pub label: String,
    pub context: String,
}

#[derive(Debug, Clone)]
pub enum VisibleItem {
    Project(usize),
    Feature(usize, usize),
    Session(usize, usize, usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SessionFilter::next ───────────────────────────────────

    #[test]
    fn session_filter_next_cycles_through_all_variants() {
        let all = SessionFilter::ALL.as_slice();
        for (i, variant) in all.iter().enumerate() {
            let next = variant.next();
            let expected = &all[(i + 1) % all.len()];
            assert_eq!(
                &next, expected,
                "after {i} expected {:?} got {:?}",
                expected, next
            );
        }
    }

    #[test]
    fn session_filter_last_wraps_to_first() {
        let last = SessionFilter::ALL.last().unwrap();
        let next = last.next();
        assert_eq!(next, SessionFilter::ALL[0]);
    }

    #[test]
    fn session_filter_all_has_six_variants() {
        assert_eq!(SessionFilter::ALL.len(), 6);
    }
}
