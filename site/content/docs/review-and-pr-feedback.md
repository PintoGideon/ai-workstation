+++
title = "Pull-Request Feedback"
description = "Working through pull-request comments with PR Triage."
weight = 50
+++

## Work through pull-request feedback

With an authenticated `gh` and a GitHub remote, a feature whose branch has a
pull request shows a `[PR #N · M open]` badge on its dashboard row and in an
embedded session's header, refreshed every few minutes in the background.
Once that PR is merged or closed without merging, the badge switches to
`[PR #N merged]` / `[PR #N closed]` instead of disappearing.

Select a feature and press `G` to open PR Triage. You can inspect review
threads, send an individual fix prompt or, with `B`, a batched one covering
several comments in a single agent run, reply, and mark threads done.
GitHub actions are presented for confirmation before AIW writes to the pull
request.

<figure>
  <img src="/images/docs/pr-triage-comments.png" alt="PR Triage listing several review comments with a detail pane showing the selected comment's diff hunk and text">
  <figcaption>PR Triage: comments on the left, the selected one's diff hunk and text on the right.</figcaption>
</figure>

A comment that asks a question rather than requesting a change can be
investigated instead of fixed: press `v` to run a strictly read-only headless
pass on the selected comment — it inspects the repo but changes nothing.
Press `e` first to attach an optional note — a hypothesis the investigation
verifies against the PR and repo. Press `a` on a finished investigation to
post an editable reply, ask a follow-up, dismiss it, or keep it as a TODO.

An unchanged AI-drafted reply discloses the harness, best-effort model,
estimated tokens, and estimated cost of the session that wrote it. When
several comments were fixed together in one `B` batch, the shared run's cost
is shown in full on
every comment it resolved, marked `combined` (`combined (3)` for a
three-comment batch) so it reads as one run's cost rather than one per
issue.

