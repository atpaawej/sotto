# Never swallow errors

Every failure is either handled deliberately or propagated to a caller. It is never discarded. In Rust that rules out `let _ =`, `.ok()`, `unwrap_or_default()`, `if let Ok(x) = ... { } else { /* nothing */ }` and `?`-less `unwrap`/`expect` on failure paths that just return early. In TypeScript it rules out empty `catch`, `.catch(() => {})`, and floating promises that nobody observes.

## Why

Typeless is a cloud-only BYOK dictation app, so almost every failure is one the user can see and fix: a bad API key, no network, a rate-limited provider, a denied microphone, a refused global-shortcut shortcut, a blocked injection. The worst outcome is not an error message — it is a pill that flashes and no text, or an app that looks idle while nothing works. A silently swallowed error turns a fixable problem into an unreportable one. Logging an error and pretending the operation succeeded counts as swallowing: the user still sees nothing.

## Consequences

- A single failure path carries every user-visible error: the pill shows a short reason, Settings keeps the full detail, and the log keeps the cause chain. Features do not invent their own ad-hoc "something went wrong" strings.
- Provider, audio, portal and injection errors are typed and mapped into that path. An error that cannot be shown or acted on is a design gap, not a reason to drop it.
- Tests and checks are never weakened to make a failure pass. A failing test is information.
- Enforcement: `clippy::let_underscore_must_use` and `clippy::unused_must_use` denied in CI, ESLint `no-empty` and a no-floating-promises rule on the UI, and code review rejects newly discarded results.

## The only exemption

Teardown where the outcome is already decided and nothing can be recovered: deleting a temp file, restoring the previous clipboard contents, shutting down an idle stream. These are marked at the call site with a `// best-effort: <why>` comment and still logged at debug level. The exemption never extends to anything that decides whether text reaches the user.
