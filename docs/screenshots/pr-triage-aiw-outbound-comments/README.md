# AIW outbound comments in PR Triage

Captured from an isolated AIW instance against a deterministic, read-only
GitHub CLI fixture. The fixture contains seven comments: five incoming or
standalone comments that remain actionable, one AIW follow-up collated beneath
its root, and one orphaned follow-up retained as context. No GitHub writes were
performed.

## 1. Collated AIW reply

The header reports only three open comments, and AIW's reply is shown under the
root comment's **Replies** section instead of appearing as a duplicate
actionable row.

![AIW reply collated under its root PR comment](001-aiw-reply-collated.png)

## 2. AIW-authored standalone finding

An AI Review finding posted by AIW remains actionable after refresh. Its
`[via AIW]` marker provides attribution without removing the normal fix, mark,
reply, and memory actions.

![AIW-authored standalone finding remains actionable](002-aiw-standalone-actionable.png)

## 3. Human reply

An unrelated human reply remains visible and retains the normal fix, mark,
reply, and memory actions.

![Human PR reply remains actionable](003-human-reply-actionable.png)
