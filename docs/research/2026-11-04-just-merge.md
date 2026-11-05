# `just merge`: merging only what CI proved green

Date: 2026-11-04, docs-only exception 2026-11-05. Ticket #42, part 2 (the gallery's "Call ended" race, part 1, follows in its own PR).
Legend: **[V]** = verified (where is stated). **[I]** = inferred, not tested.

**Bottom line:** `just merge <pr>` squash-merges a PR only once every check passed on a head that already contains `origin/main`, and GitHub reports it mergeable. Commits the head lacks may change only `docs/` and `*.md` files, unless code, tests or CI name one of them. Each refusal names the condition and the next command. Shown refusing a behind PR, a pending check and a failed check on throwaway PR #45, and letting a docs-only commit through but not a code one on throwaway PR #59.

## 1. What it checks

- Branch protection and rulesets return 403 on this private Free repository (ticket #42), so the recipe is the only guard. `tools/merge.py` refuses unless, in order: the PR is open; its head contains the tip of `origin/<base>` (GitHub's compare API, `behind_by` = 0), or lacks only commits that change docs (§2); every check on the head passed, after waiting for pending ones with `gh pr checks --watch` (30 minutes by default, `--wait MINUTES` to change, `--wait 0` to refuse at once); the head did not move while waiting, and still contains `origin/<base>`; GitHub reports it mergeable and not a draft. Then `gh pr merge --squash --match-head-commit <head>` and a `DELETE` of the branch ref on GitHub, so local branches and worktrees are left alone. **[V]** for the refusals in §2; **[I]** for the rest.
- Exit status: 0 merged and branch deleted; 1 refused, nothing merged; 2 merged, but the branch remains.

## 2. Findings

- A workflow run that is still queued has no check yet: `gh pr checks` says `no checks reported`, though `gh run list --commit <sha>` shows the run as `pending`. With six PRs in flight the runners queued for minutes. The recipe counts such runs as pending checks, so it waits for them and does not mistake a queued run for "no CI". If nothing at all is reported for 3 minutes, it refuses. **[V]** (PR #45)
- Straight after `gh pr update-branch`, `gh pr view` still returned the old head for a moment, so the recipe refused the PR as behind once more. A refusal on stale data is the safe direction; a few seconds later the compare API saw the new head 0 behind. **[V]** (PR #45)
- On a terminal `gh pr checks --watch` redraws its table in place; anywhere else it prints the whole table every 10 s. The recipe hides that table when its output is not a terminal, such as an agent's Bash tool. **[V]** (the long output below, before the change)

- The chief of staff pushes wiki and glossary commits to main between PRs, so "contains `origin/main`" alone would refuse nearly every PR. The recipe lets through missing commits whose files (the compare API's net file list, renames counted by both names) are all under `docs/` or end in `.md`, and prints one line naming them. **[V]** (PR #59, §4)
- Docs are not always untested: `crates/automation/tests/scorecard.rs` reads `docs/wiki/polish-scorecard.md`, and `conformance.rs` reads `docs/research/2026-11-04-dbus-snapshot/`. So a docs path still refuses if any file outside `docs/` and `*.md`, in the base branch or the PR head, names it or a directory holding it (`git grep` after fetching both). Comments count too, which can only refuse a safe merge; dated research notes are rarely edited. `GLOSSARY.md` and most of the wiki are named nowhere. **[V]** (the two paths above are reported as named, `GLOSSARY.md` and `docs/wiki/index.md` are not, checked against PR #56's head)
- If the compare API's lists are truncated (250 commits, 300 files), the recipe refuses as plainly behind. **[I]**

## 3. Refusals shown on throwaway PR #45

PR #45 was a draft branched from the commit before `origin/main`'s tip, with a line that fails `cargo fmt --check`. It was closed unmerged and its branch deleted. This output predates the docs-only exception; the behind refusal now names what the missing commits change (§4).

Behind `origin/main` (exit 1):

```
$ just merge 45
Not merged: PR #45 is 1 commit behind origin/main, so its CI did not test what main will become.
Next: gh pr update-branch 45   (then `just merge 45` waits for the new checks)
```

After `gh pr update-branch 45`, with CI's run queued and no check reported yet (exit 1):

```
$ just merge 45 --wait 0
Not merged: 1 of 1 checks on PR #45 is still pending without waiting:
  CI (pull_request): pending
Next: just merge 45   (it waits for them)
```

Waiting for that check, which failed on `cargo fmt --check` (exit 1; `gh`'s check table, which this run still printed every 10 s, is elided):

```
$ just merge 45
Waiting up to 30 min for checks on PR #45:
Not merged: 1 of 1 checks on PR #45 did not pass:
  just check (pull_request): failure  https://github.com/danielbaldwin47/desktop-calls/actions/runs/41965977375/job/127043280604
Next: gh run view 41965977375 --log-failed
```

Not exercised, because no PR may be merged in this ticket: the draft, conflict and "head moved" refusals, and the merge and branch deletion themselves. **[I]**

## 4. The docs-only exception on throwaway PR #59

PR #59 was a draft with one throwaway commit, closed unmerged, its branch deleted and its CI runs cancelled. `origin/main` was `4f4cccf` (Contacts sync, code) then `3338d6d` (`GLOSSARY.md` only).

Branched from the commit before both, so it lacks a code commit (exit 1):

```
$ just merge 59 --wait 0
Not merged: PR #59 lacks 2 commits on origin/main that change crates/core/src/contacts.rs and 22 other files, so its CI did not test what main will become.
Next: gh pr update-branch 59   (then `just merge 59` waits for the new checks)
```

After merging `4f4cccf` into it, so it lacks only the glossary commit; the next condition, a pending check, then refuses (exit 1):

```
$ just merge 59 --wait 0
Letting through 1 docs-only commit on origin/main that PR #59 lacks: 3338d6d Glossary: Helper, the person on the other phone during a Real-Phone Checklist
Not merged: 1 of 1 checks on PR #59 is still pending without waiting:
  CI (pull_request): pending
Next: just merge 59   (it waits for them)
```

The refusal for a docs path that code names was checked at the function only (§2), as no such commit is on main. **[I]** for its printed message.
