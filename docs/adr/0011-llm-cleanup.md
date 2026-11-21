# Cleanup uses a fast second LLM call

Between the Transcript and Injection, Sotto runs a second fast LLM call on Groq (`qwen/qwen3.8-27b` by default, reusing the user's Groq API key) rather than a deterministic regex rulebook. This supersedes [ADR-0003](0003-deterministic-cleanup.md).

## Why

A local rulebook cannot resolve mid-sentence self-corrections, infer natural punctuation and capitalization from phrasing, or fix phonetic mishearings of Dictionary terms without breaking real words. Code does not understand language; a small, fast instruct model with thinking disabled (`reasoning_effort: "none"`, `temperature: 0`) handles fillers, punctuation, self-corrections, and Dictionary contextually while keeping total latency minimal.

## Cleanup levels

- `none` — Skips the second call and injects the raw Transcript.
- `light` (default) — Removes fillers and stutters, fixes punctuation and casing, resolves self-corrections, and applies Dictionary terms while preserving the user's exact phrasing.
- `medium` — Applies light cleanup plus light rewording for clarity and conciseness.

## Consequences

- Cleanup reuses `groq_key` and `cleanup.model` (`qwen/qwen3.8-27b` by default) over a persistent HTTP client to avoid extra TCP/TLS handshakes.
- Dictionary entries (`Vec<String>`) are sent both as Whisper's `prompt` hint and in the Cleanup LLM's system prompt; there is no code-based regex dictionary pass.
- Per [ADR-0001](0001-never-swallow-errors.md), if the Cleanup LLM call fails, Sotto injects the raw Whisper Transcript as-is (never running a fallback regex pass) and surfaces the Cleanup failure on the Pill, in Settings, and in the log.
- Both the raw Transcript and the cleaned text are preserved on each `Attempt`.
