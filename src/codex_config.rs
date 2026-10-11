use std::path::Path;
use std::process::Command;

use crate::project::VibeMode;

pub fn launch_override_args(workdir: &Path, mode: &VibeMode) -> Vec<String> {
    let hook_path = workdir.join(".codex").join("aiw-codex-notify.sh");
    let user_config_path = dirs::home_dir()
        .map(|home| home.join(".codex").join("config.toml"))
        .unwrap_or_default();

    launch_override_args_for(&user_config_path, &hook_path, workdir, mode)
}

pub fn configured_model() -> Option<String> {
    let config_path = dirs::home_dir()?.join(".codex").join("config.toml");
    configured_model_for(&config_path)
}

fn configured_model_for(config_path: &Path) -> Option<String> {
    read_launch_config(config_path)?
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToOwned::to_owned)
}

/// Model slugs visible in Codex's own model picker, from sources that are
/// cheap to read synchronously: the catalog cache first, falling back to the
/// older availability table for installs that predate the cache. Deliberately
/// excludes the `codex debug models` CLI probe — that shells out to the
/// `codex` binary and can block for as long as the process takes to exit (or
/// hang, on a stuck or network-blocked install), so it must never run on the
/// TUI's event-loop thread. Callers that want the CLI-sourced catalog spawn
/// [`spawn_cli_catalog_probe`] on a background thread instead.
///
/// No-ops under `cfg!(test)`, so unit tests never depend on the machine's
/// real `~/.codex/config.toml`.
pub fn known_models() -> Vec<String> {
    if cfg!(test) {
        return Vec::new();
    }
    let Some(codex_home) = dirs::home_dir().map(|home| home.join(".codex")) else {
        return Vec::new();
    };
    let cache_path = codex_home.join("models_cache.json");
    known_models_from_cache(&cache_path)
        .unwrap_or_else(|| known_models_for(&codex_home.join("config.toml")))
}

/// Spawn a background thread that asks the installed Codex CLI for its model
/// catalog and reports the result on the returned channel. This is the only
/// caller of [`known_models_from_cli`]: it keeps the (potentially slow or
/// hanging) `Command::output()` call off the TUI's event-loop thread, mirroring
/// every other headless call in this codebase, which is spawned on a
/// background thread and polled rather than awaited inline.
pub fn spawn_cli_catalog_probe() -> std::sync::mpsc::Receiver<Option<Vec<String>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(known_models_from_cli());
    });
    rx
}

/// Read the catalog used by Codex's own model picker. The NUX table in
/// config.toml only records models that have already been surfaced to a user;
/// the cache contains the complete visible catalog and is therefore the
/// correct source for an AIW picker.
fn known_models_from_cache(cache_path: &Path) -> Option<Vec<String>> {
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(cache_path).ok()?).ok()?;
    known_models_from_catalog_value(&value)
}

/// Ask the installed Codex CLI for its catalog when its cache has not been
/// written yet. This is the same catalog rendered by Codex's model picker and
/// keeps a fresh AIW install from falling back to `Custom…` only.
fn known_models_from_cli() -> Option<Vec<String>> {
    let output = Command::new("codex")
        .args(["debug", "models"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    known_models_from_catalog_value(&value)
}

fn known_models_from_catalog_value(value: &serde_json::Value) -> Option<Vec<String>> {
    // The picker needs names only; a catalog entry does not prove account access.
    let mut models: Vec<String> = value
        .get("models")?
        .as_array()?
        .iter()
        .filter(|model| model.get("visibility").and_then(|v| v.as_str()) == Some("list"))
        .filter_map(|model| model.get("slug")?.as_str())
        .filter(|slug| {
            !slug.is_empty()
                && !slug.starts_with('-')
                && !slug.chars().any(|c| c.is_whitespace() || c.is_control())
        })
        .map(str::to_string)
        .collect();
    models.sort();
    models.dedup();
    Some(models)
}

fn known_models_for(config_path: &Path) -> Vec<String> {
    let Some(models) = read_launch_config(config_path).and_then(|table| {
        table
            .get("tui")?
            .as_table()?
            .get("model_availability_nux")?
            .as_table()
            .cloned()
    }) else {
        return Vec::new();
    };
    let mut models: Vec<String> = models.into_iter().map(|(key, _)| key).collect();
    models.sort();
    models
}

fn launch_override_args_for(
    config_path: &Path,
    hook_path: &Path,
    workdir: &Path,
    mode: &VibeMode,
) -> Vec<String> {
    let user_config = read_launch_config(config_path).unwrap_or_default();
    let launch_config = build_launch_config(&user_config, hook_path);
    let mut args = vec![
        "-C".to_string(),
        workdir.to_string_lossy().into_owned(),
        "--add-dir".to_string(),
        workdir.to_string_lossy().into_owned(),
    ];
    if matches!(mode, VibeMode::SuperVibe) {
        args.extend([
            "--sandbox".to_string(),
            "danger-full-access".to_string(),
            "--ask-for-approval".to_string(),
            "never".to_string(),
        ]);
    }
    args.extend(launch_config_to_args(launch_config));
    args
}

fn read_launch_config(config_path: &Path) -> Option<toml::map::Map<String, toml::Value>> {
    let config = std::fs::read_to_string(config_path)
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())?;

    config.as_table().cloned()
}

fn build_launch_config(
    user_config: &toml::map::Map<String, toml::Value>,
    hook_path: &Path,
) -> toml::map::Map<String, toml::Value> {
    let mut launch_config = toml::map::Map::new();
    launch_config.insert(
        "notify".to_string(),
        build_notify_override(user_config.get("notify"), hook_path),
    );
    launch_config
}

// FORK: Codex's `notify` is one argv (program + args; Codex appends the JSON
// payload), not a list of commands. AIW never writes `~/.codex/config.toml`;
// each launch passes `notify = [<worktree hook>, <user's own notify argv>...]`
// with `-c`, and the hook replays the user's command (see
// scripts/codex-notify.sh). Entries AIW itself once wrote into the global
// file are dropped so they don't run twice.
fn build_notify_override(notify: Option<&toml::Value>, hook_path: &Path) -> toml::Value {
    let hook_cmd = hook_path.to_string_lossy().into_owned();
    let user_argv: Vec<String> = parse_notify_entries(notify)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| !is_aiw_notify_hook(entry))
        .collect();
    let mut argv = vec![hook_cmd];
    argv.extend(user_argv);
    toml::Value::Array(argv.into_iter().map(toml::Value::String).collect())
}

/// AIW's own notify hooks: the per-worktree `.codex/aiw-codex-notify.sh` and
/// `<config dir>/codex-notify.sh`. FORK: upstream AMF's equivalents
/// (`.codex/amf-codex-notify.sh`, `<config root>/amf/codex-notify.sh`, which
/// older AMF versions added to `~/.codex/config.toml`) count too.
fn is_aiw_notify_hook(entry: &str) -> bool {
    let path = Path::new(entry);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    matches!(name, "aiw-codex-notify.sh" | "amf-codex-notify.sh")
        || (name == "codex-notify.sh"
            && matches!(
                parent,
                crate::project::AIW_DIR_NAME | crate::project::LEGACY_AMF_DIR_NAME
            ))
}

fn parse_notify_entries(value: Option<&toml::Value>) -> Option<Vec<String>> {
    let Some(value) = value else {
        return Some(vec![]);
    };

    if let Some(arr) = value.as_array() {
        let values: Option<Vec<String>> = arr
            .iter()
            .map(|item| item.as_str().map(ToOwned::to_owned))
            .collect();
        return values;
    }

    value.as_str().map(|command| vec![command.to_string()])
}

fn launch_config_to_args(config: toml::map::Map<String, toml::Value>) -> Vec<String> {
    let mut args = Vec::new();
    for (key, value) in config {
        args.push("-c".to_string());
        args.push(format!("{key}={value}"));
    }
    args
}

/// Append launch-specific evidence guidance without writing shared configuration.
/// Codex's CLI developer_instructions override has precedence over file layers.
pub(crate) fn with_screenshot_guidance(
    workdir: &Path,
    guidance: &str,
    args: Vec<String>,
) -> anyhow::Result<Vec<String>> {
    let global = if cfg!(test) {
        None
    } else {
        std::env::var_os("CODEX_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))
            .map(|p| p.join("config.toml"))
    };
    let mut paths: Vec<_> = global.into_iter().collect();
    let mut ancestors: Vec<_> = workdir.ancestors().collect();
    ancestors.reverse();
    paths.extend(ancestors.into_iter().map(|p| p.join(".codex/config.toml")));
    append_guidance_from_configs(&paths, guidance, args)
}

fn append_guidance_from_configs(
    paths: &[std::path::PathBuf],
    guidance: &str,
    mut args: Vec<String>,
) -> anyhow::Result<Vec<String>> {
    fn merge(target: &mut toml::Value, source: toml::Value) {
        if let (Some(dest), Some(layer)) = (target.as_table_mut(), source.as_table()) {
            for (key, value) in layer {
                if let Some(existing) = dest.get_mut(key) {
                    merge(existing, value.clone());
                } else {
                    dest.insert(key.clone(), value.clone());
                }
            }
        } else {
            *target = source;
        }
    }
    let mut config = toml::Value::Table(toml::map::Map::new());
    for path in paths {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        merge(&mut config, toml::from_str(&content)?);
    }
    let mut profile = config
        .get("profile")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    for (index, arg) in args.iter().enumerate() {
        if matches!(arg.as_str(), "-p" | "--profile") {
            profile = args.get(index + 1).cloned();
        } else if let Some(value) = arg.strip_prefix("--profile=") {
            profile = Some(value.into());
        }
    }
    let mut effective = config
        .get("developer_instructions")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    if let Some(profile) = profile
        && let Some(instructions) = config
            .get("profiles")
            .and_then(|v| v.get(&profile))
            .and_then(|v| v.get("developer_instructions"))
            .and_then(|v| v.as_str())
    {
        effective = instructions.into();
    }
    for pair in args.windows(2) {
        if matches!(pair[0].as_str(), "-c" | "--config")
            && let Some(value) = pair[1].strip_prefix("developer_instructions=")
        {
            effective = toml::from_str::<toml::Value>(&format!("v={value}"))
                .ok()
                .as_ref()
                .and_then(|parsed| parsed.get("v"))
                .and_then(|v| v.as_str())
                .unwrap_or(value)
                .into();
        }
    }
    if !effective.is_empty() {
        effective.push_str("\n\n");
    }
    effective.push_str(guidance);
    args.extend([
        "-c".into(),
        format!("developer_instructions={}", toml::Value::String(effective)),
    ]);
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::launch_override_args_for;
    use crate::project::VibeMode;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn launch_override_args_merges_existing_user_entries() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        let hook_path = dir.path().join("aiw-codex-notify.sh");

        fs::write(&config_path, "notify = [\"/tmp/existing-hook.sh\"]\n").unwrap();
        let args = launch_override_args_for(&config_path, &hook_path, dir.path(), &VibeMode::Vibe);

        assert_eq!(args.len(), 6);
        assert_eq!(args[0], "-C");
        assert_eq!(args[1], dir.path().to_string_lossy());
        assert_eq!(args[2], "--add-dir");
        assert_eq!(args[3], dir.path().to_string_lossy());
        assert_eq!(args[4], "-c");
        assert!(
            args[5].contains("/tmp/existing-hook.sh"),
            "existing user notify entry should be preserved"
        );
        assert!(
            args[5].contains("aiw-codex-notify.sh"),
            "AIW hook should be injected into the transient override"
        );
    }

    // FORK: the notify hook is passed per launch and the user's config is
    // only read, never written.
    #[test]
    fn notify_override_puts_hook_first_and_chains_user_argv() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        let hook_path = dir.path().join(".codex").join("aiw-codex-notify.sh");
        let original = "model = \"gpt-5\"\nnotify = [\"notify-send\", \"Codex done\", \"/home/u/.config/ai_workstation/codex-notify.sh\"]\n";
        fs::write(&config_path, original).unwrap();

        let args = launch_override_args_for(&config_path, &hook_path, dir.path(), &VibeMode::Vibe);
        let notify = args.last().unwrap();
        let value: toml::Value = toml::from_str(notify).unwrap();
        let argv: Vec<&str> = value["notify"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            argv,
            vec![
                hook_path.to_string_lossy().as_ref(),
                "notify-send",
                "Codex done"
            ]
        );
        assert_eq!(fs::read_to_string(&config_path).unwrap(), original);
    }

    #[test]
    fn notify_override_drops_stale_aiw_entries() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        let hook_path = dir
            .path()
            .join("wt")
            .join(".codex")
            .join("aiw-codex-notify.sh");
        fs::write(
            &config_path,
            // FORK: upstream AMF's global and per-worktree hooks are dropped too.
            "notify = [\"/home/u/.config/ai_workstation/codex-notify.sh\", \"/old/wt/.codex/aiw-codex-notify.sh\", \"/home/u/.config/amf/codex-notify.sh\", \"/old/wt/.codex/amf-codex-notify.sh\"]\n",
        )
        .unwrap();
        let args = launch_override_args_for(&config_path, &hook_path, dir.path(), &VibeMode::Vibe);
        let value: toml::Value = toml::from_str(args.last().unwrap()).unwrap();
        assert_eq!(
            value["notify"].as_array().unwrap().len(),
            1,
            "only this launch's hook should remain: {value:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn notify_hook_replays_the_users_notify_command_with_the_payload() {
        let dir = TempDir::new().unwrap();
        let hook = dir.path().join("aiw-codex-notify.sh");
        fs::write(&hook, include_str!("../scripts/codex-notify.sh")).unwrap();
        let out = dir.path().join("out.txt");
        let user = dir.path().join("user.sh");
        fs::write(
            &user,
            format!(
                "#!/bin/sh\nprintf '%s|%s' \"$1\" \"$2\" > '{}'\n",
                out.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&user, fs::Permissions::from_mode(0o755)).unwrap();

        let status = std::process::Command::new("bash")
            .arg(&hook)
            .arg(&user)
            .arg("tag")
            .arg("{\"type\":\"agent-turn-complete\"}")
            .env_remove("AIW_ACTIVE")
            .status()
            .unwrap();
        assert!(status.success());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !out.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            fs::read_to_string(&out).unwrap(),
            "tag|{\"type\":\"agent-turn-complete\"}"
        );
    }

    #[test]
    fn launch_override_args_falls_back_to_aiw_hook_only() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("missing.toml");
        let hook_path = dir.path().join("aiw-codex-notify.sh");

        let args = launch_override_args_for(&config_path, &hook_path, dir.path(), &VibeMode::Vibe);

        assert_eq!(args.len(), 6);
        assert_eq!(args[0], "-C");
        assert_eq!(args[1], dir.path().to_string_lossy());
        assert_eq!(args[2], "--add-dir");
        assert_eq!(args[3], dir.path().to_string_lossy());
        assert_eq!(args[4], "-c");
        assert!(args[5].contains("aiw-codex-notify.sh"));
    }

    #[test]
    fn configured_model_reads_top_level_model() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(&config_path, "model = \"gpt-5.5\"\n").unwrap();

        assert_eq!(
            super::configured_model_for(&config_path).as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn known_models_for_reads_tui_model_availability_nux_table() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(
            &config_path,
            "model = \"gpt-5.6-terra\"\n\n[tui.model_availability_nux]\n\"gpt-5.6-sol\" = 4\n\"gpt-5.5\" = 4\n",
        )
        .unwrap();

        assert_eq!(
            super::known_models_for(&config_path),
            vec!["gpt-5.5".to_string(), "gpt-5.6-sol".to_string()]
        );
    }

    #[test]
    fn known_models_for_is_empty_without_the_nux_table() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(&config_path, "model = \"gpt-5.6-terra\"\n").unwrap();

        assert!(super::known_models_for(&config_path).is_empty());
    }

    #[test]
    fn known_models_for_is_empty_when_config_is_missing() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("missing.toml");

        assert!(super::known_models_for(&config_path).is_empty());
    }

    #[test]
    fn known_models_from_cache_reads_visible_catalog_slugs() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("models_cache.json");
        fs::write(
            &cache_path,
            r#"{"models":[{"slug":"gpt-5.6-terra","visibility":"list"},{"slug":"hidden","visibility":"hide"},{"slug":"gpt-5.5","visibility":"list"},{"slug":"gpt-5.5","visibility":"list"}]}"#,
        )
        .unwrap();

        assert_eq!(
            super::known_models_from_cache(&cache_path),
            Some(vec!["gpt-5.5".to_string(), "gpt-5.6-terra".to_string()])
        );
    }

    #[test]
    fn launch_override_args_supervibe_uses_full_access_without_approvals() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("missing.toml");
        let hook_path = dir.path().join("aiw-codex-notify.sh");

        let args =
            launch_override_args_for(&config_path, &hook_path, dir.path(), &VibeMode::SuperVibe);

        assert!(
            args.windows(2)
                .any(|pair| pair == ["--sandbox", "danger-full-access"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--ask-for-approval", "never"])
        );
    }
}

#[cfg(test)]
mod screenshot_tests {
    use super::*;
    #[test]
    fn screenshot_guidance_preserves_profiles_across_file_layers() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("global.toml");
        let local = dir.path().join("local.toml");
        std::fs::write(&global, "developer_instructions='Global'\n[profiles.review]\ndeveloper_instructions='Review conventions'\nmodel='my-model'\n").unwrap();
        std::fs::write(
            &local,
            "profile='review'\n[profiles.review]\nmodel='local-model'\n",
        )
        .unwrap();
        let paths = [global, local];
        for args in [vec![], vec!["--profile".into(), "review".into()]] {
            let result = append_guidance_from_configs(&paths, "Capture rule", args).unwrap();
            assert!(result.last().unwrap().contains("Review conventions"));
        }
        assert!(
            std::fs::read_to_string(&paths[1])
                .unwrap()
                .contains("local-model")
        );
    }
    #[test]
    fn evidence_guidance_preserves_raw_cli_instruction_overrides() {
        for flag in ["-c", "--config"] {
            for value in [
                "Follow our conventions",
                "Unmatched \"quote",
                "'Quoted instructions'",
            ] {
                let args = append_guidance_from_configs(
                    &[],
                    "Evidence",
                    vec![flag.into(), format!("developer_instructions={value}")],
                )
                .unwrap();
                let parsed: toml::Value = toml::from_str(
                    args.last()
                        .unwrap()
                        .strip_prefix("developer_instructions=")
                        .map(|value| format!("v={value}"))
                        .unwrap()
                        .as_str(),
                )
                .unwrap();
                let expected = if value == "'Quoted instructions'" {
                    "Quoted instructions"
                } else {
                    value
                };
                assert_eq!(
                    parsed["v"].as_str(),
                    Some(format!("{expected}\n\nEvidence").as_str())
                );
            }
        }
    }

    #[test]
    fn evidence_guidance_preserves_config_and_cli_instructions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = "developer_instructions = 'Keep my conventions'\nmodel = 'test-model'\n";
        std::fs::write(&path, original).unwrap();
        let args = append_guidance_from_configs(
            std::slice::from_ref(&path),
            "Capture only when explicitly requested",
            vec![],
        )
        .unwrap();
        assert!(args[1].contains("Keep my conventions"));
        assert!(args[1].contains("explicitly requested"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let args = append_guidance_from_configs(
            &[path],
            "Evidence",
            vec![
                "-c".into(),
                "developer_instructions=\"CLI instructions\"".into(),
            ],
        )
        .unwrap();
        assert!(args.last().unwrap().contains("CLI instructions"));
    }
}
