use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::project::AgentKind;
use serde::{Deserialize, Serialize};

/// Harness-agnostic entry point for one-shot, non-interactive agent work.
///
/// Prompts are always piped over stdin. Besides keeping secrets and large
/// prompts out of process listings, this avoids Linux's 128 KiB per-argument
/// limit for real-world PR diffs.
pub struct HeadlessRunner;

// ---------------------------------------------------------------------------
// Overflow-error classification
//
// The hard backstop behind the soft size gate: when a harness rejects a
// prompt as too long *despite* the estimate clearing the budget, the run
// fails with a typed [`PromptTooLong`] instead of a generic error string so
// a caller can tell it apart from auth or network failures. Any failure the classifier does not recognize stays a
// plain `anyhow` error: the CLIs' "too long" phrasings are unverified across
// versions, so the classifier is deliberately conservative.
// ---------------------------------------------------------------------------

/// A one-shot run rejected because the prompt exceeded the model's context
/// window. Carried through `anyhow` so a caller can `downcast` / use
/// [`as_prompt_too_long`] to tell it apart from auth, quota, and network
/// failures, which no amount of splitting would fix.
#[derive(Debug, Clone)]
pub struct PromptTooLong {
    pub harness: AgentKind,
    /// The provider/CLI text that was classified as an overflow, trimmed.
    pub detail: String,
}

impl std::fmt::Display for PromptTooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} rejected the prompt as too long for its context window{}{}",
            self.harness.display_name(),
            if self.detail.is_empty() { "" } else { ": " },
            self.detail
        )
    }
}

impl std::error::Error for PromptTooLong {}

/// Build the typed error for `harness` from the classified `detail` text.
pub fn prompt_too_long_error(harness: &AgentKind, detail: &str) -> anyhow::Error {
    anyhow::Error::new(PromptTooLong {
        harness: harness.clone(),
        detail: detail.trim().to_string(),
    })
}

/// The [`PromptTooLong`] in `err`'s chain, if any — set even when the error
/// has since been wrapped with extra `.context(...)`.
#[allow(dead_code)]
pub fn as_prompt_too_long(err: &anyhow::Error) -> Option<&PromptTooLong> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<PromptTooLong>())
}

/// Best-effort detection of a "prompt exceeds the context window" failure in a
/// harness's error text (stderr on non-zero exit, or a structured event
/// error). Matches the phrasings each CLI/provider is known to emit:
/// Anthropic/Claude Code, OpenAI/Codex, and the assorted providers Pi
/// routes to. An unrecognized phrasing falls through and surfaces as an
/// ordinary failure until it is verified against that CLI.
pub fn is_prompt_too_long_message(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        // Anthropic / Claude Code
        "prompt is too long",
        "prompt too long",
        // OpenAI / Codex
        "maximum context length",
        "context_length_exceeded",
        "reduce the length of the messages",
        // Generic provider phrasings (Pi model routing)
        "context length exceeded",
        "context window exceeded",
        "exceeds the context window",
        "exceed the context window",
        "input is too long",
        "input too long",
        "too many tokens",
        "too many input tokens",
    ];
    if NEEDLES.iter().any(|needle| lowered.contains(needle)) {
        return true;
    }
    // "... 210000 tokens > 200000 maximum ..." style, where no fixed phrase
    // is present but the shape is unambiguous.
    lowered.contains("tokens >") && lowered.contains("maximum")
}

#[derive(Debug, PartialEq, Eq)]
struct HeadlessCommand {
    binary: String,
    args: Vec<&'static str>,
    /// Args that must stay last (e.g. Codex's trailing `-` stdin marker). An
    /// optional `--model <name>` is inserted between `args` and `trailing`.
    trailing: Vec<&'static str>,
    /// Extra env vars for the spawned process.
    envs: Vec<(&'static str, &'static str)>,
}

impl HeadlessRunner {
    /// `model`, when set, is passed as an explicit `--model <name>` (the
    /// flag name/format every harness shares) so a caller can pick a model
    /// independent of whatever the feature's interactive session runs.
    ///
    /// `restricted`, when true, forces Claude and Pi into a no-tools
    /// invocation that repo-controlled config (settings files, hooks,
    /// plugins, MCP servers) cannot loosen — for callers whose prompt
    /// carries all the context it needs and expects a plain text answer, not
    /// a repo-exploring agent run. Codex is always sandboxed read-only
    /// already.
    pub fn run<'a>(
        harness: &AgentKind,
        workdir: &Path,
        prompt: &str,
        model: impl Into<ModelSel<'a>>,
        restricted: bool,
    ) -> Result<String> {
        let spec = command_for(harness, restricted);
        run_command(harness, &spec, workdir, prompt, model.into())
    }
}

/// How hard a headless run should think, as one vocabulary over four
/// harnesses that each spell it differently. Not every harness accepts every
/// level: [`ReasoningLevel::supported_for`] is the verified set, and
/// [`reasoning_args`] drops a level the harness cannot express rather than
/// passing a value its CLI would reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningLevel {
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
}

impl ReasoningLevel {
    /// The levels a harness's headless CLI accepts, weakest first. Verified
    /// against `claude --help` (`--effort`: low, medium, high, xhigh, max),
    /// and the Codex config key `model_reasoning_effort`. Pi has no verified
    /// flag, so it offers none.
    pub fn supported_for(harness: &AgentKind) -> &'static [ReasoningLevel] {
        use ReasoningLevel::*;
        match harness {
            AgentKind::Claude => &[Low, Medium, High, XHigh, Max],
            AgentKind::Codex => &[Low, Medium, High, XHigh],
            AgentKind::Pi => &[],
            AgentKind::Gemini | AgentKind::Cursor => &[], // FORK
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// The model and reasoning level a headless run was asked for. Every entry
/// point takes `impl Into<ModelSel>`, so a caller with only a model (an
/// `Option<&str>`) is unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelSel<'a> {
    pub model: Option<&'a str>,
    pub reasoning: Option<ReasoningLevel>,
}

impl<'a> ModelSel<'a> {}

impl<'a> From<Option<&'a str>> for ModelSel<'a> {
    fn from(model: Option<&'a str>) -> Self {
        Self {
            model,
            reasoning: None,
        }
    }
}

/// The CLI args that request `level` from `harness`, or none when the harness
/// cannot express it.
pub fn reasoning_args(harness: &AgentKind, level: ReasoningLevel) -> Vec<String> {
    if !ReasoningLevel::supported_for(harness).contains(&level) {
        return Vec::new();
    }
    let slug = level.slug();
    match harness {
        AgentKind::Claude => vec!["--effort".to_string(), slug.to_string()],
        AgentKind::Codex => vec![
            "-c".to_string(),
            format!("model_reasoning_effort=\"{slug}\""),
        ],
        AgentKind::Pi => Vec::new(),
        AgentKind::Gemini | AgentKind::Cursor => Vec::new(), // FORK
    }
}

/// Harnesses whose headless CLI accepts `--model <name>`.
fn supports_model_flag(harness: &AgentKind) -> bool {
    match harness {
        AgentKind::Claude | AgentKind::Codex | AgentKind::Pi => true,
        AgentKind::Gemini | AgentKind::Cursor => false, // FORK
    }
}

/// [`assemble_args`] with runtime-built `extra` args (e.g. a user's MCP
/// config path) placed after `spec.args` and before `--model`/`trailing`.
fn assemble_args_with(
    harness: &AgentKind,
    spec: &HeadlessCommand,
    extra: &[String],
    sel: ModelSel<'_>,
) -> Vec<String> {
    let mut args: Vec<String> = spec.args.iter().map(|arg| arg.to_string()).collect();
    args.extend(extra.iter().cloned());
    if let Some(model) = sel.model
        && supports_model_flag(harness)
    {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    if let Some(level) = sel.reasoning {
        args.extend(reasoning_args(harness, level));
    }
    args.extend(spec.trailing.iter().map(|arg| arg.to_string()));
    args
}

fn run_command(
    harness: &AgentKind,
    spec: &HeadlessCommand,
    workdir: &Path,
    prompt: &str,
    sel: ModelSel<'_>,
) -> Result<String> {
    run_command_with_args(harness, spec, &[], workdir, prompt, sel)
}

fn run_command_with_args(
    harness: &AgentKind,
    spec: &HeadlessCommand,
    extra: &[String],
    workdir: &Path,
    prompt: &str,
    sel: ModelSel<'_>,
) -> Result<String> {
    // FORK: Gemini/Cursor have no headless contract yet.
    if crate::fork::extra_agents::is_extra(harness) {
        return Err(headless_unsupported(harness));
    }
    // Held for the whole run so the concurrency gate sees headless work.
    // Taken here rather than at each call site: every path out of this
    // function — spawn failure, `?`, cancellation, panic — releases it.
    let _lease = crate::resources::limits::HeadlessLease::acquire();
    let args = assemble_args_with(harness, spec, extra, sel);

    let mut child = Command::new(&spec.binary)
        .args(&args)
        .envs(spec.envs.iter().copied())
        .current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "Failed to spawn {} in headless mode",
                harness.display_name()
            )
        })?;

    // Drain output concurrently with the write. A large response can fill
    // stdout before the harness consumes all of a large prompt, otherwise
    // deadlocking a write-then-wait implementation.
    let mut stdin = child
        .stdin
        .take()
        .with_context(|| format!("Failed to open stdin for {}", harness.display_name()))?;
    let prompt = prompt.to_string();
    let writer = std::thread::spawn(move || stdin.write_all(prompt.as_bytes()));

    let output = child
        .wait_with_output()
        .with_context(|| format!("Failed to run {} headlessly", harness.display_name()))?;
    let write_result = writer
        .join()
        .map_err(|_| anyhow::anyhow!("{} prompt writer panicked", harness.display_name()))?;

    // Prefer the provider's exit error over a secondary BrokenPipe from the
    // writer: authentication/quota failures often make a CLI exit before it
    // consumes a large prompt, and its stderr is the actionable diagnosis.
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        if is_prompt_too_long_message(detail) {
            return Err(prompt_too_long_error(harness, detail));
        }
        anyhow::bail!(
            "{} headless command failed ({}){}{}",
            harness.display_name(),
            output.status,
            if detail.is_empty() { "" } else { ": " },
            detail
        );
    }
    write_result.with_context(|| format!("Failed to send prompt to {}", harness.display_name()))?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// FORK: Gemini/Cursor are interactive-only in ACC for now.
fn headless_unsupported(harness: &AgentKind) -> anyhow::Error {
    anyhow::anyhow!(
        "{} has no headless mode in AIW yet; pick Claude, Codex or Pi for this",
        harness.display_name()
    )
}

fn command_for(harness: &AgentKind, restricted: bool) -> HeadlessCommand {
    match harness {
        // FORK: never spawned — every runner rejects these harnesses first.
        AgentKind::Gemini | AgentKind::Cursor => HeadlessCommand {
            binary: harness.slug().to_string(),
            args: vec![],
            trailing: vec![],
            envs: vec![],
        },
        AgentKind::Claude => {
            let mut args = vec!["-p", "--output-format", "text"];
            if restricted {
                // Verified against the installed `claude --help`: --safe-mode
                // drops repo-configured hooks/MCP servers/plugins without
                // touching auth (unlike --bare, which also disables OAuth),
                // and --tools "" empties the tool whitelist outright so no
                // settings-file allow rule can grant Bash/Edit/Read.
                args.extend(["--safe-mode", "--tools", ""]);
            }
            HeadlessCommand {
                binary: crate::claude::ClaudeLauncher::resolve_binary(),
                args,
                trailing: vec![],
                envs: vec![],
            }
        }
        AgentKind::Codex => HeadlessCommand {
            binary: "codex".into(),
            args: vec![
                "exec",
                "--sandbox",
                "read-only",
                "--ephemeral",
                "--skip-git-repo-check",
                "--color",
                "never",
            ],
            // Codex reads the prompt from stdin only when `-` is the final
            // positional arg; an inserted `--model` must land before it.
            trailing: vec!["-"],
            envs: vec![],
        },
        AgentKind::Pi => {
            let mut args = vec!["-p", "--no-session"];
            if restricted {
                // --no-tools covers built-in, extension, and custom tools on
                // current Pi releases. Disable all discovered prompt/code
                // resources as a second boundary, and ignore project-local
                // configuration so a repository cannot weaken the run.
                args.extend([
                    "--no-tools",
                    "--no-extensions",
                    "--no-skills",
                    "--no-prompt-templates",
                    "--no-context-files",
                    "--no-approve",
                ]);
            }
            HeadlessCommand {
                binary: "pi".into(),
                args,
                trailing: vec![],
                envs: vec![],
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::limits::{lock_lease_tests, wait_for_in_flight};

    // FORK: Gemini/Cursor are refused before anything is spawned.

    #[test]
    fn a_failed_headless_spawn_releases_its_lease() {
        let _guard = lock_lease_tests();
        // Wait for any lease leaked by an earlier test to drain before reading
        // the baseline. A run abandoned elsewhere in the suite (closing the
        // changeset-overview modal, say) keeps its lease for
        // `ABANDONED_RUN_GRACE` past the end of that test, so capturing the
        // count directly can record a baseline that then falls away underneath
        // the assertion below.
        let base = wait_for_in_flight(0);
        let spec = HeadlessCommand {
            binary: "aiw-no-such-headless-binary".into(),
            args: vec!["-p"],
            trailing: vec![],
            envs: vec![],
        };
        let result = run_command(
            &AgentKind::Claude,
            &spec,
            std::path::Path::new("/tmp"),
            "prompt",
            ModelSel::default(),
        );
        assert!(result.is_err(), "spawn of a missing binary must fail");
        assert_eq!(wait_for_in_flight(base), base);
    }

    #[test]
    fn every_agent_harness_has_a_headless_command() {
        let claude = command_for(&AgentKind::Claude, false);
        assert!(!claude.binary.is_empty());
        assert_eq!(claude.args, ["-p", "--output-format", "text"]);

        assert_eq!(
            command_for(&AgentKind::Codex, false),
            HeadlessCommand {
                binary: "codex".into(),
                args: vec![
                    "exec",
                    "--sandbox",
                    "read-only",
                    "--ephemeral",
                    "--skip-git-repo-check",
                    "--color",
                    "never",
                ],
                trailing: vec!["-"],
                envs: vec![],
            }
        );
        assert_eq!(
            command_for(&AgentKind::Pi, false),
            HeadlessCommand {
                binary: "pi".into(),
                args: vec!["-p", "--no-session"],
                trailing: vec![],
                envs: vec![],
            }
        );
    }

    #[test]
    fn restricted_harnesses_deny_repo_configurable_tool_access() {
        let claude = command_for(&AgentKind::Claude, true);
        assert!(claude.args.contains(&"--safe-mode"));
        let tools_idx = claude
            .args
            .iter()
            .position(|arg| *arg == "--tools")
            .expect("--tools flag present");
        assert_eq!(claude.args[tools_idx + 1], "");

        // Codex is already sandboxed unconditionally; restricted is a no-op.
        assert_eq!(
            command_for(&AgentKind::Codex, true),
            command_for(&AgentKind::Codex, false)
        );

        let pi = command_for(&AgentKind::Pi, true);
        for flag in [
            "--no-tools",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-context-files",
            "--no-approve",
        ] {
            assert!(pi.args.contains(&flag), "restricted Pi lacks {flag}");
        }
    }

    #[test]
    fn runner_pipes_the_prompt_over_stdin() {
        let spec = HeadlessCommand {
            binary: "sh".into(),
            args: vec!["-c", "read input; printf 'received:%s' \"$input\""],
            trailing: vec![],
            envs: vec![],
        };
        let output = run_command(
            &AgentKind::Codex,
            &spec,
            Path::new("/tmp"),
            "hello",
            ModelSel::default(),
        )
        .expect("fake headless command should succeed");
        assert_eq!(output, "received:hello");
    }

    #[test]
    fn runner_failure_names_the_provider_and_preserves_stderr() {
        let spec = HeadlessCommand {
            binary: "sh".into(),
            args: vec!["-c", "printf 'quota exhausted' >&2; exit 9"],
            trailing: vec![],
            envs: vec![],
        };
        // Large enough that the early exit also breaks the stdin writer. The
        // provider's stderr must win over that secondary pipe error.
        let prompt = "x".repeat(1_000_000);
        let error = run_command(
            &AgentKind::Pi,
            &spec,
            Path::new("/tmp"),
            &prompt,
            ModelSel::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("Pi headless command failed"));
        assert!(error.contains("quota exhausted"));
    }

    #[test]
    fn model_flag_is_appended_for_supported_harnesses_only() {
        assert!(supports_model_flag(&AgentKind::Claude));
        assert!(supports_model_flag(&AgentKind::Codex));
        assert!(supports_model_flag(&AgentKind::Pi));
    }

    #[test]
    fn reasoning_level_maps_to_each_harness_flag() {
        use ReasoningLevel::*;
        assert_eq!(
            reasoning_args(&AgentKind::Claude, XHigh),
            ["--effort", "xhigh"]
        );
        assert_eq!(
            reasoning_args(&AgentKind::Codex, High),
            ["-c", "model_reasoning_effort=\"high\""]
        );
    }

    #[test]
    fn reasoning_level_a_harness_cannot_express_is_dropped() {
        use ReasoningLevel::*;
        assert!(reasoning_args(&AgentKind::Codex, Max).is_empty());
        assert!(reasoning_args(&AgentKind::Claude, Minimal).is_empty());
        assert!(reasoning_args(&AgentKind::Pi, High).is_empty());
    }

    #[test]
    fn reasoning_level_serializes_as_its_slug() {
        for level in ReasoningLevel::supported_for(&AgentKind::Claude) {
            let json = serde_json::to_string(level).unwrap();
            assert_eq!(json, format!("\"{}\"", level.slug()));
            assert_eq!(
                serde_json::from_str::<ReasoningLevel>(&json).unwrap(),
                *level
            );
        }
    }

    #[test]
    fn classifies_known_overflow_messages_for_every_harness() {
        // Anthropic / Claude Code (`claude -p`)
        assert!(is_prompt_too_long_message("Prompt is too long"));
        assert!(is_prompt_too_long_message(
            "API Error: 400 {\"type\":\"invalid_request_error\",\"message\":\"prompt is too long: 210000 tokens > 200000 maximum\"}"
        ));
        // OpenAI / Codex (`codex exec`)
        assert!(is_prompt_too_long_message(
            "This model's maximum context length is 272000 tokens. However, your messages resulted in 401000 tokens. Please reduce the length of the messages."
        ));
        assert!(is_prompt_too_long_message(
            "Error code: 400 - {'error': {'code': 'context_length_exceeded'}}"
        ));
        // Providers Pi routes to
        assert!(is_prompt_too_long_message(
            "input is too long for requested model"
        ));
        // Generic phrasing that could come back from Pi
        assert!(is_prompt_too_long_message(
            "the request exceeds the context window for this model"
        ));
        // The bare "N tokens > M maximum" shape with no fixed phrase.
        assert!(is_prompt_too_long_message(
            "got 500000 tokens > 200000 maximum"
        ));
    }

    #[test]
    fn does_not_classify_unrelated_failures_as_overflow() {
        assert!(!is_prompt_too_long_message(""));
        assert!(!is_prompt_too_long_message(
            "401 Unauthorized: invalid API key"
        ));
        assert!(!is_prompt_too_long_message(
            "rate limit exceeded, please retry after 20s"
        ));
        assert!(!is_prompt_too_long_message(
            "error: could not connect to the model provider"
        ));
        assert!(!is_prompt_too_long_message(
            "the diff exceeds the maximum number of lines"
        ));
    }

    #[test]
    fn prompt_too_long_error_survives_anyhow_context_wrapping() {
        let err = prompt_too_long_error(
            &AgentKind::Codex,
            "  maximum context length is 272000 tokens  ",
        );
        let typed = as_prompt_too_long(&err).expect("typed error at the root");
        assert_eq!(typed.harness, AgentKind::Codex);
        assert_eq!(typed.detail, "maximum context length is 272000 tokens");

        let wrapped = err.context("batch 3 of 12 failed");
        assert!(
            as_prompt_too_long(&wrapped).is_some(),
            "wrapping with .context() must not hide the typed cause"
        );
    }

    #[test]
    fn a_plain_failure_is_not_seen_as_prompt_too_long() {
        let err = anyhow::anyhow!("Codex headless command failed: 401 Unauthorized");
        assert!(as_prompt_too_long(&err).is_none());
    }
}
