//! Narrow GUI bridge for local evidence. Scan/decode outside the GuiHandle lock.
use crate::gui_contract::{FeatureTarget, GuiError, GuiHandle, GuiResult};
pub use crate::screenshot_evidence::{
    EvidenceIssue, EvidenceItem, EvidenceListing, EvidenceOwner, ImageData,
};
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EvidenceSelection {
    pub feature: Option<FeatureTarget>,
    pub session_id: Option<String>,
}

pub struct EvidenceRead {
    owners: Vec<EvidenceOwner>,
}
impl EvidenceRead {
    pub fn run(self) -> EvidenceListing {
        crate::screenshot_evidence::scan(self.owners)
    }
}

pub fn plan(gui: &mut GuiHandle, selection: EvidenceSelection) -> GuiResult<EvidenceRead> {
    gui.refresh_store()?;
    let app = gui.app_for_workflow();
    if let Some(target) = &selection.feature {
        let (pi, fi) = app
            .store
            .locate_feature_by_id(Some(&target.project_id), &target.feature_id)
            .ok_or_else(|| GuiError::not_found("Feature was deleted; open retained screenshots"))?;
        let _ = (pi, fi);
    }
    let db = app
        .db
        .as_ref()
        .ok_or_else(|| GuiError::not_found("No evidence database attached"))?;
    // External worktree deletion invalidates owned scopes, never retained root evidence.
    for (owner, _) in db.evidence_scopes(false).map_err(GuiError::from)? {
        if owner.is_worktree && !owner.workdir.exists() {
            db.retire_evidence(&owner.scope_id)
                .map_err(GuiError::from)?;
        }
    }
    let owners: Vec<_> = db
        .evidence_scopes(false)
        .map_err(GuiError::from)?
        .into_iter()
        .map(|(o, _)| o)
        .filter(|o| {
            selection
                .feature
                .as_ref()
                .is_none_or(|f| f.project_id == o.project_id && f.feature_id == o.feature_id)
                && selection
                    .session_id
                    .as_ref()
                    .is_none_or(|s| s == &o.session_id)
        })
        .collect();
    app.evidence_work.watch(&owners);
    Ok(EvidenceRead { owners })
}

pub fn finish(gui: &mut GuiHandle, mut listing: EvidenceListing) -> GuiResult<EvidenceListing> {
    let db = gui.db()?;
    let mut active = std::collections::HashSet::new();
    for owner in &listing.owners {
        if owner.is_worktree && !owner.workdir.exists() {
            db.retire_evidence(&owner.scope_id)
                .map_err(GuiError::from)?;
            continue;
        }
        if db
            .evidence_scope_active(&owner.scope_id)
            .map_err(GuiError::from)?
        {
            active.insert(owner.scope_id.clone());
        }
    }
    listing.items.retain(|i| active.contains(&i.scope_id));
    listing.issues.retain(|i| active.contains(&i.scope_id));
    listing.owners.retain(|i| active.contains(&i.scope_id));
    Ok(listing)
}

pub fn changed(gui: &mut GuiHandle) -> bool {
    gui.app_for_workflow().evidence_work.changed()
}

pub struct ImageRead {
    owner: EvidenceOwner,
    image_id: String,
    hash: String,
    thumbnail: bool,
}
impl ImageRead {
    pub fn run(&self) -> GuiResult<ImageData> {
        let _permit = crate::screenshot_evidence::image_worker().map_err(GuiError::from)?;
        crate::screenshot_evidence::load_image(
            &self.owner,
            &self.image_id,
            &self.hash,
            self.thumbnail,
        )
        .map_err(GuiError::from)
    }
}
pub fn plan_image(
    gui: &GuiHandle,
    scope_id: String,
    image_id: String,
    hash: String,
    thumbnail: bool,
) -> GuiResult<ImageRead> {
    let owner = gui
        .db()?
        .evidence_scopes(false)
        .map_err(GuiError::from)?
        .into_iter()
        .find(|(o, _)| o.scope_id == scope_id)
        .map(|(o, _)| o)
        .ok_or_else(|| GuiError::conflict("Evidence was cleaned up; refresh screenshots"))?;
    Ok(ImageRead {
        owner,
        image_id,
        hash,
        thumbnail,
    })
}
pub fn finish_image(gui: &GuiHandle, read: &ImageRead) -> GuiResult<()> {
    if read.owner.is_worktree && !read.owner.workdir.exists() {
        gui.db()?
            .retire_evidence(&read.owner.scope_id)
            .map_err(GuiError::from)?;
    }
    if !gui
        .db()?
        .evidence_scope_active(&read.owner.scope_id)
        .map_err(GuiError::from)?
    {
        return Err(GuiError::conflict(
            "Evidence was cleaned up; refresh screenshots",
        ));
    }
    Ok(())
}

/// Includes retired scopes so a failed removal can be retried without live records.
pub fn cleanup_scopes(gui: &GuiHandle) -> GuiResult<Vec<(EvidenceOwner, bool)>> {
    gui.db()?.evidence_scopes(true).map_err(GuiError::from)
}

pub fn plan_cleanup(gui: &mut GuiHandle, scope_id: &str) -> GuiResult<CleanupRead> {
    let db = gui.db()?;
    let owner = db
        .evidence_scopes(true)
        .map_err(GuiError::from)?
        .into_iter()
        .find(|(o, _)| o.scope_id == scope_id)
        .map(|(o, _)| o)
        .ok_or_else(|| GuiError::not_found("Screenshot scope no longer exists"))?;
    db.retire_evidence(scope_id).map_err(GuiError::from)?;
    Ok(CleanupRead { owner })
}

// FORK(no-cloud): AIW's remote screenshot sources (GitHub Actions artifact
// galleries, PR-body images fetched over HTTPS) and the image-access checker
// were removed — AIW downloads nothing itself. What remains is the local
// evidence above, the PR description (one `gh` call when the user opens a
// PR's description in PR Triage), and handing a link to the user's browser.

/// The selected PR in PR Triage, as the description read needs it.
#[derive(Debug, Clone)]
pub(crate) struct PrContext {
    pub workflow_id: String,
    pub feature_id: String,
    pub workdir: std::path::PathBuf,
    pub owner: String,
    pub repo: String,
    pub number: u32,
    pub head_sha: String,
}

/// Inline PR reads never replace the shared artifact/gallery selection.
pub struct PrDocumentRead {
    context: PrContext,
}
impl PrDocumentRead {
    pub fn run(&self) -> GuiResult<String> {
        // Explicit user action (opening the PR description): one `gh` call.
        let meta = crate::github::GhCli::pr_meta(&self.context.workdir, self.context.number)
            .map_err(GuiError::from)?;
        Ok(meta.body)
    }
}
pub fn plan_pr_document(gui: &mut GuiHandle, workflow_id: &str) -> GuiResult<PrDocumentRead> {
    Ok(PrDocumentRead {
        context: crate::gui_pr_triage::screenshot_context(gui, workflow_id)?,
    })
}
pub fn finish_pr_document(gui: &mut GuiHandle, read: &PrDocumentRead) -> GuiResult<()> {
    fresh_inline(gui, &read.context)
}

fn fresh_inline(gui: &mut GuiHandle, context: &PrContext) -> GuiResult<()> {
    let current = crate::gui_pr_triage::screenshot_context(gui, &context.workflow_id)?;
    if current.number != context.number
        || current.head_sha != context.head_sha
        || current.feature_id != context.feature_id
        || current.owner != context.owner
        || current.repo != context.repo
    {
        return Err(GuiError::conflict("PR image context changed"));
    }
    Ok(())
}

/// Hand an `http(s)` link to the user's browser. AIW itself fetches nothing.
pub fn open_gallery_browser(url: &str) -> GuiResult<()> {
    let parsed = url::Url::parse(url)
        .ok()
        .filter(|u| {
            url.len() <= 8192
                && matches!(u.scheme(), "http" | "https")
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
        })
        .ok_or_else(|| GuiError::conflict("Unsupported or unsafe link"))?;
    let command = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(command)
        .arg(parsed.as_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| GuiError::from(anyhow::Error::from(e)))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub struct CleanupRead {
    owner: EvidenceOwner,
}
impl CleanupRead {
    pub fn run(self) -> GuiResult<()> {
        crate::screenshot_evidence::remove_owned_directory(&self.owner).map_err(GuiError::from)
    }
}
pub fn cleanup(gui: &mut GuiHandle, scope_id: &str) -> GuiResult<()> {
    plan_cleanup(gui, scope_id)?.run()
}
