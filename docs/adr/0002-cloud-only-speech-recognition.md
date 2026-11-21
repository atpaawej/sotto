# Cloud-only speech recognition in v1

Typeless ships with no local speech model: every Dictation is transcribed by a cloud provider chosen by the user, using the user's own API key. The `Transcriber` seam exists so a local adapter can be added later, but v1 has no offline mode and cannot produce text without a network and a key.

## Why

Local models cost the thing this app is judged on: the first run. whisper.cpp needs a model download (75 MB to 1.5 GB), warms up slowly on CPU, and transcribes noisier real-world speech worse than the hosted models. A small binary that gives the best accuracy on the first sentence is worth more than an offline guarantee that most users will never exercise.

## Considered options

- **Local default, cloud opt-in** — honest open-source story, but a large first-run download, a cold-start tax on every session, and a hardware floor.
- **Local only** — a hard offline guarantee, at the cost of accuracy and binary size.
- **Cloud only, BYOK** — what we chose.

## Consequences

- Onboarding cannot end without a working key. The first-run wizard tests the key with a real request before it lets the user finish.
- Audio always leaves the machine. The privacy promise is limited to *local storage*: Recordings and Transcripts are kept on disk and never uploaded anywhere except to the configured provider.
- Every failure mode we cannot reproduce locally (401, 429, 5xx, timeout) is a real user-facing state, which is why failures have a single surfacing path (see ADR-0001).
- Adding a local adapter later must not change the `DictationSession` interface, and must not make the app slower for cloud users.
