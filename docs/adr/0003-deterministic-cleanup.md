# Cleanup is deterministic, not a language model

> **Superseded by [ADR-0011](0011-llm-cleanup.md).**

Between the Transcript and Injection, the text passes through a fixed rulebook: filler removal, whitespace repair, spoken punctuation, and Dictionary corrections. No language model rewrites the text in v1, and there is no "cleanup level" setting — only a master switch and one switch per rule.

## Why

The competitor-comparison evidence is unambiguous: users of LLM-rewriting dictation tools complain that the tool "improves" what they said, and the top cleanup level was withdrawn because it over-edited. A rulebook can be explained in four lines, tested exhaustively, runs in under two milliseconds, and can never invent a sentence the user did not say. That property is worth more than smoother prose.

## Considered options

- **Layered: rules plus an LLM pass, on by default** — the polish is what these products sell, but the failure mode is silent and unbounded.
- **Rules only (chosen)** — predictable and fast; occasionally clunky.
- **Raw Transcript** — a transcript full of "um" and no punctuation is not usable writing.

## Consequences

- Cleanup is a module with a tiny interface (`cleanup(transcript, dictionary, rules) -> String`), not a seam: there is one adapter, so a trait would be speculative.
- The raw Transcript is always stored alongside the cleaned text, so the rulebook is never destructive: anything it removes is recoverable.
- Adding an optional LLM pass later is a new function behind the same interface, plus a setting; it must keep the raw Transcript stored and must remain off by default.
- Anything needing judgement (self-corrections, "twenty three" to 23, list formatting) is out of scope until the rules are proven in daily use.
