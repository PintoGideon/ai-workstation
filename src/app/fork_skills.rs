//! FORK: the agent-skill catalogue the skill picker searches (DESIGN.md
//! §18). These helpers came out of the compose box when it went: they read
//! `.claude/skills` / `.agents/skills` manifests and score names, and have
//! nothing to do with typing.

use std::path::Path;

use crate::app::state::ComposeCommandEntry;

/// Characters that begin a new "word" inside a command name, so a
/// query character landing right after one is a strong match. Covers
/// namespace separators (`stn:commit`) and the usual word boundaries.
fn is_command_boundary(c: char) -> bool {
    matches!(c, ':' | '-' | '_' | '/' | ' ' | '.')
}

/// Fuzzy-match `query` against `candidate`, both compared case-
/// insensitively. Returns a score (higher is better) when every
/// character of `query` appears in order within `candidate`, or `None`
/// when it does not. An empty query matches everything neutrally.
///
/// Scoring favors matches at the start of the name or just after a
/// namespace/word boundary and rewards consecutive runs, so typing
/// `commit` surfaces `stn:commit` while still ranking a literal
/// `commit` command above it.
pub(crate) fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let query_lower: Vec<char> = query
        .chars()
        .flat_map(|c| c.to_lowercase())
        .filter(|c| !c.is_whitespace())
        .collect();
    if query_lower.is_empty() {
        return Some(0);
    }

    let candidate_chars: Vec<char> = candidate.chars().collect();
    let mut score = 0i32;
    let mut q = 0usize;
    let mut prev_match: Option<usize> = None;

    for (idx, ch) in candidate_chars.iter().enumerate() {
        if q >= query_lower.len() {
            break;
        }
        let cand_lower = ch.to_lowercase().next().unwrap_or(*ch);
        if cand_lower != query_lower[q] {
            continue;
        }

        score += 1;
        if idx == 0 {
            score += 8;
        } else if is_command_boundary(candidate_chars[idx - 1]) {
            score += 6;
        }
        if idx > 0 && prev_match == Some(idx - 1) {
            score += 4;
        }

        prev_match = Some(idx);
        q += 1;
    }

    if q == query_lower.len() {
        // Prefer tighter candidates so closer matches sort first.
        Some(score - (candidate_chars.len() as i32) / 8)
    } else {
        None
    }
}

/// The available agent skills — global (`~/.claude/skills`) plus the
/// project's own (`{workdir}/.claude/skills`) — deduped by name with the
/// project copy winning. Same scan the compose catalog uses for skills, but
/// without the slash-command entries, for the prompt-library skill picker.
pub(crate) fn build_skill_catalog(workdir: Option<&Path>) -> Vec<ComposeCommandEntry> {
    let mut catalog: Vec<ComposeCommandEntry> = Vec::new();
    let push_unique = |catalog: &mut Vec<ComposeCommandEntry>, entry: ComposeCommandEntry| {
        if !catalog
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&entry.name))
        {
            catalog.push(entry);
        }
    };

    // Project skills first so they win the name collision over global ones.
    if let Some(workdir) = workdir {
        for entry in scan_skills_dir(&workdir.join(".claude").join("skills")) {
            push_unique(&mut catalog, entry);
        }
    }
    if let Some(home) = dirs::home_dir() {
        for entry in scan_skills_dir(&home.join(".claude").join("skills")) {
            push_unique(&mut catalog, entry);
        }
    }

    catalog.sort_by(|a, b| a.name.cmp(&b.name));
    catalog
}

fn scan_skills_dir(dir: &Path) -> Vec<ComposeCommandEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut skills: Vec<ComposeCommandEntry> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.is_file() {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            let description = frontmatter_description(&skill_md).unwrap_or_default();
            Some(ComposeCommandEntry { name, description })
        })
        .collect();

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Extract the `description:` value from a markdown file's YAML
/// frontmatter, if present.
fn frontmatter_description(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            return None;
        }
        if let Some(value) = trimmed.strip_prefix("description:") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if value.is_empty() {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}
