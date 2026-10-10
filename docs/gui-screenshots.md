# Viewing screenshots in the GUI

PR Triage and PR Review show the **PR description**, selected comment and
thread replies. AIW (this fork) downloads no images itself: an image in PR
Markdown is shown as a placeholder with a button that opens it in your browser
(see [NO_CLOUD.md](../NO_CLOUD.md)).

PR links open in your browser. Reading a comment does not build a separate
image gallery or automatically download Actions artifacts or linked galleries.
The description has its own retry if GitHub cannot be read.

Use **Screenshots** on a feature to review agent validation evidence, or
**Session screenshots** to filter to the selected producing session. These
entries also work for stopped features and ordinary non-git directories.
The workspace's **Validation screenshots** entry includes historical evidence
after the original sessions/features disappear.

Click a thumbnail to open the shared viewer. Use **Fit**, **Original size**,
**Zoom in/out**, and drag or scroll to pan. **Previous/Next** or the arrow keys
move between images. Escape returns to the gallery, then closes the gallery;
your PR comment selection and unsent reply stay in place. **Retry image** retries
a failed remote read where it appears.

New Claude and Codex launches receive a producing-session destination and
completion instructions. Those instructions permit capture only when you
explicitly request visual validation. Other harnesses do not receive this
initial guidance. Existing running sessions need a new launch to receive it.
Images require a completion manifest with ownership and a hash; unfinished
images remain in the gallery's incomplete-evidence diagnostics. The exact
formats and publication steps are in the
[evidence contract](development/screenshot-evidence-contract.md).

Worktree deletion removes its screenshot files. Failed deletion retains the
evidence. Repository-root and non-git screenshots remain until you choose
**Screenshot cleanup…**, select a producing scope, and confirm **Delete
screenshots**. Cleanup also remains accessible after the feature is gone. A
partial removal can be retried. Restart the producing agent after cleanup to
receive a fresh destination.

PNG, JPEG, WebP and GIF are supported for local validation screenshots;
animations show their first frame. AIW's GitHub image and Actions-artifact
downloader (and its **Check access** button) is not part of AIW.
