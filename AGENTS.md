# Typeless

An open-source desktop dictation app for Linux. Hold a key, speak, and clean text lands where your cursor is.

## Hard boundaries

### Never swallow an error

Every failure is handled deliberately or propagated. Never discard one — no `let _ =`, `.ok()`, `unwrap_or_default()` to paper over a failure, empty `catch`, `.catch(() => {})`, or "log it and continue as if it worked". If nobody can act on a failure, surface it: the pill shows it, Settings keeps the detail, the log keeps the cause. This applies to every agent and every human in this repo. See `docs/adr/0001-never-swallow-errors.md`.

Never weaken a test, assertion, or type check to make a failure go away.

## Agent skills

### Issue tracker

Issues live as GitHub issues in `atpaawej/typeless`, managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` plus `docs/adr/` at the repo root. Read both before working. See `docs/agents/domain.md`.
