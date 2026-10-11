# No-cloud build (G3)

**Principle:** AI Workstation (AIW) itself makes no network connections. It is
a harness and tooling around the agents; the agents (Claude, Codex, Gemini,
Cursor, Pi, …) talk to their own providers, and that is their business.

What enforces it:

- `scripts/check-no-network.sh` fails if the root package's normal dependency
  tree (`cargo +1.99.0 tree -e normal -p ai-workstation --locked`) contains
  `ureq`, `reqwest`, `hyper`, `axum`, `tungstenite`, `h2`, `rustls`,
  `native-tls`, `openssl`, `curl`, `isahc`, `surf`, `attohttpc` (or a family
  member such as `hyper-util`, `rustls-webpki`, `tokio-tungstenite`), or
  `tokio` with its `net` feature; or if `src/` uses `std::net`, `TcpStream`,
  `TcpListener`, `UdpSocket`, `Command::new("curl")` or `Command::new("wget")`.
- `aiw doctor` reports the `network` finding: "AI Workstation
  (AIW) makes no network connections itself".

The check covers the root package (the `aiw` binary and the library).
The Tauri desktop shell (`gui/src-tauri`) links Tauri/WebKit, which are not
checked here; its own Rust code has no network calls after this change.

## Audit

Inventory of every network-capable path found in `src/` and
`gui/src-tauri/src` before this change, and what happened to it.

### Removed

| Path (as it was) | What it did | Decision |
|---|---|---|
| `src/upgrade.rs`, the upstream `upgrade` subcommand (`src/cli.rs`) | Queried the GitHub releases API and downloaded/installed release bundles over HTTPS (`ureq`). | **Removed.** Update by pulling the fork and rebuilding. |
| `src/http_client.rs` | Shared `ureq` HTTPS agent (rustls, platform verifier) and a probe agent. | **Removed** with all its users. |
| `src/usage.rs` (`fetch_rate_limits`) | Every 60 s read `~/.claude/.credentials.json` and called `https://api.anthropic.com/api/oauth/usage` with the user's Claude OAuth token to fill the Claude 5h/7d usage bars. | **Removed.** Claude usage windows now stay empty unless a local source reports them; Codex windows (read from local session files) are unchanged. |
| `src/remote_server.rs` | Remote Control companion-app server: tokio + axum HTTP/WebSocket server (`TcpListener`), `/health`, `/status`, `/actions`, pairing exchange, PWA assets. | **Removed.** |
| `src/remote_terminal.rs` | Streamed a tmux pane to the phone over WebSocket and forwarded its input. | **Removed.** |
| `src/remote_web/` (`index.html`, `app.js`, `app.css`, `sw.js`, manifest, icons) | The phone PWA served by the server. | **Removed.** |
| `src/remote_push.rs`, `src/app/remote_push.rs` | Web Push (VAPID-signed, encrypted) to browser vendors' push services (FCM, Mozilla, Apple) via `ureq`. | **Removed.** |
| `src/tailscale.rs`, `src/app/remote_tailscale.rs` | Probed the local Tailscale CLI/daemon and ran `tailscale serve` to publish the server on the tailnet. | **Removed.** |
| `src/app/remote_reach.rs` | HTTP `/health` probe of the pairing address from this machine (`ureq`; tests used `std::net::TcpListener`). | **Removed.** |
| `src/qr.rs`, `src/handlers/remote_pairing.rs`, `src/ui/dialogs/remote_pairing.rs`, `RemotePairing` mode in `src/app/state.rs` | Pairing dialog: one-time code + terminal QR (`qrcode`), paired-devices list, Tailscale setup walkthrough. | **Removed.** |
| `src/app/remote_server.rs`, `remote_actions.rs`, `remote_attention.rs`, `remote_todos.rs` | App-side lifecycle, status snapshots, phone actions/TODOs for the server. | **Removed.** |
| `src/db/remote_devices.rs`, `src/db/remote_push.rs` and their `AiwDb` accessors | Paired devices, push subscriptions, VAPID key. | **Code removed.** Migrations 042/043 are kept, so an existing `aiw.db` opens unchanged; the tables just go unused. |
| `AppConfig` keys `remote_bind`, `remote_public_url`, `remote_tailscale_cli`, `remote_tailscale_socket` | Server/tunnel settings. | **Removed.** Old config files still load (unknown keys are ignored). |
| Dashboard/session leader keys `C` ("Remote server on / off") and `Q` ("Pair a phone"), their help and legend entries, the doctor `remote-access` finding | UI for the above. | **Removed.** Doctor has a `network` finding instead. |
| `src/screenshot_sources.rs` | PR screenshot viewer's remote sources: GitHub Actions artifacts (zip download + `scraper` HTML parsing), PR-body/comment images over HTTPS (`ureq`, redirects, `std::net::IpAddr` checks), image-access checker. | **Removed.** Local agent-validation screenshots (`src/screenshot_evidence.rs`) still work. |
| `src/gui_screenshots.rs` remote half; Tauri commands `screenshots_remote_list`, `screenshots_remote_image`, `screenshots_remote_close`, `screenshots_inline_image`, `screenshots_check_access`; `gui/src/PrMarkdown.tsx` inline image fetch | GUI bridge for the above. | **Removed.** PR Markdown images are placeholders with an "Open in browser" button. |
| `tungstenite` in `src/app/model_analysis/session_control.rs` | WebSocket framing for the Codex settings control socket of the model analyzer. | **Removed** with the model analyzer (2026-10-11); no WebSocket library is linked at all, not even in tests. |
| Background PR badge sweep (`ACTIVE_PR_SYNC_INTERVAL`, `src/cli.rs` main loop + startup, `src/gui_contract/sidebar.rs` GUI timer) | `gh` GraphQL queries for every project's open/merged PRs every 5 minutes and once at startup. | **Off.** Runs only on explicit refresh (`R`, leader `r` on the dashboard or in a session, `r` on a project row, the `refresh-statuses` command) or when PR Triage opens (TUI `G`, GUI PR Triage). Otherwise the last-known badge state is shown (merged/closed state is cached in SQLite). |
| Startup retry of pending issue comments (`App::new` → `queue_recoverable_issue_comments`) | Posted an "I'm working on this" comment via `gh` at every start if an earlier post hadn't completed. | **Moved** to the explicit refresh (`R`). |
| Tree-sitter startup validation (`src/highlight/tree_sitter.rs`) | A parser that failed validation at startup was automatically re-downloaded (`git clone` from GitHub) and rebuilt. | **No auto-download.** Startup only logs "parser needs repair: open the syntax picker (p) and press i to re-download it". |

Dependencies removed from the root package: `ureq`, `tokio`, `axum`, `qrcode`,
`web-push-native`, `p256`, `zip`, `scraper`, `tungstenite`, `sha1`. Their transitive network stack (hyper, hyper-util, h2,
rustls, rustls-webpki, rustls-platform-verifier, tokio-tungstenite,
openssl-probe, …) is gone with them: the root package went from 312 to 189
linked packages.

### Kept (and why)

| Path | What it does | Why it stays |
|---|---|---|
| `src/github.rs` `GhCli` (20 `Command::new("gh")` sites) | PR resolve/list/meta/diff/comments/reviews/threads, posting reviews/replies/comments, resolving threads, issue list/comment, current user. | Every call is behind a user action: PR Triage/Review (`G`), AI Review (`W`), review memory bootstrap, issue fixer (`g`), posting, explicit refresh. `gh` is the user's own authenticated CLI. |
| `git fetch` in `src/github.rs` (local diff fallback), `src/app/pr_review/revisions.rs` (PR revisions), `src/app/triage_feature.rs` (PR head); `git push` in `src/app/triage_feature.rs` (land triage commits on the PR) | Fetch PR refs for review; push when the user lands commits. | Explicit user actions in PR Triage/Review. |
| Tree-sitter parser install (`src/highlight/tree_sitter.rs`, `src/app/syntax.rs`, `src/gui_syntax.rs`) | `git clone` of a grammar repo, then `cc`. | Only when the user picks it (syntax picker `p` → `i`/Enter, `i` in the diff viewer/triage, GUI "Install … parser…" with a confirmation that says it clones from GitHub). Labelled as a download: "Downloading (git clone) + building", badge `DOWNLOADING`, help "Download + install …". |
| `gui_screenshots::PrDocumentRead` | PR description for the GUI (`gh pr view` via `GhCli::pr_meta`). | Runs when the user opens a PR in PR Triage. |
| `open_gallery_browser` (`xdg-open`/`open`) | Hands an `http(s)` link to the user's browser. | AIW fetches nothing; the browser does, on a click. |
| Claude `--remote-control` launch flag (`src/project.rs`) | Claude Code's own Remote Control bridge to claude.ai. | The agent's own network feature, opt-in per feature; AIW only passes the flag. The URL copy/open and pane parsing (`src/app/remote_control.rs`) were removed with the embedded view (2026-10-11, DESIGN.md §18). |
| `src/ipc.rs`, tmux sockets | Unix-domain sockets. | Local IPC, not network. |
| Harness hooks and Claude's status line (`scripts/*.sh` → `aiw permission-request`, `status-line`, `stop-hook`, `subagent`, `session-context`; DESIGN.md §16) | The agent CLIs run AIW's scripts on their own events; the scripts talk to AIW's Unix socket and print a line or a JSON decision back to the agent. The status line restores the Claude 5h/7d meters from the numbers Claude itself passes to the command. | Local processes and the local socket; nothing of AIW's opens a network connection. |
| Agents' own traffic | Claude, Codex, Gemini, Cursor, Pi, … | Outside AIW. |

Not product code: the dev harness for the removed remote screenshot sources
(`scripts/dev/check-gui-screenshots.py`, `scripts/dev/fixtures/screenshot-gh.py`) and the
remote-control PWA capture (`scripts/dev/screenshot/capture-remote-dashboard.cjs` and its
`remote-dashboard-collapse` scenario) were deleted in the 2026-10-10 cleanup. Left as is:
`docs/backlog/remote-control-companion-app-plan.md`
and `CHANGELOG.md` (history).
