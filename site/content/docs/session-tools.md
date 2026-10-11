+++
title = "More Session Tools"
description = "Bookmarks and resuming a harness's saved transcripts."
weight = 37
+++

A few more tools for moving between sessions and managing how much history
each one is carrying, all reachable from the dashboard.

## Bookmark a session

Press `Ctrl+Space`, then `H` to bookmark the selected session into the next
open slot — there are nine. `Ctrl+Space`, then `h` opens the bookmark
picker, showing all nine slots and what's in each; `Ctrl+Space`, then a digit
`1`–`9` jumps straight to that slot's session. `Ctrl+Space`, then `M`
removes the selected session's bookmark. Once all nine slots are full,
bookmarking a new session evicts the oldest one. A slot pointing at a
session that's since been deleted clears itself the next time you try to
jump to it.

## Resume a harness's own saved transcripts

Press `S` on a Claude or Codex feature or session to browse that
harness's own saved conversation history and resume one in a new AIW
session — including transcripts from before AIW started tracking the
feature, and even for a stopped feature (the picker starts it for you). Pi
has no saved-session storage AIW can read, so `S` isn't offered there. See
[Choosing a Harness](@/docs/harnesses.md) for the rest of what varies by
harness.

## Icons for custom sessions

When adding a custom session (`s`, or the config wizard's Custom Sessions
category — see [Configuration](@/docs/configuration.md)), pick from 14
curated Nerd Font icons — Server, Database, Docker, and so on — or type any
glyph or emoji of your own.
