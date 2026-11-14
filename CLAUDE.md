## Agent skills

### Issue tracker

Issues live in GitHub Issues for `danielbaldwin47/desktop-calls`, managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

### Shared machine

One CI lane and other agents' processes: read `docs/agents/shared-machine.md` before running `just ci-local` or killing anything.

## Coding standards

Every code change follows [`CODING_STANDARDS.md`](CODING_STANDARDS.md). Run `just check` before pushing; CI runs the same.
