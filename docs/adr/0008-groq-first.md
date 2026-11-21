# Groq is the provider that ships first

The first provider wired up is Groq's Whisper `large-v3-turbo`, not ElevenLabs Scribe. `active_provider` defaults to Groq, and a config written by the walking-skeleton build moves to Groq on load when it never held an ElevenLabs key. ElevenLabs stays in the config schema, unwired, for ticket #8 to finish.

## Why

This was a scope call, not a verdict on quality: Groq was asked for, it needs one key and one multipart request, and its turbo model answers a ten-second recording in about 300 ms — measured, not assumed (413 ms from key release to text, all stages included). That leaves the latency budget free for the parts of the product that are actually hard: the gesture, the pill, injection.

Scribe v3 may still be the better transcriber. Nothing in the seam assumes otherwise: both are adapters behind `Transcriber`, and moving the default back is a config change plus one adapter.

## Consequences

- A build with an ElevenLabs provider selected says so plainly instead of pretending: `Dictation::start` refuses with "ElevenLabs is not wired up in this build", which the window shows.
- The migration only switches a file that had no ElevenLabs key. A user who pasted one keeps their choice, and their key is not deleted.
- `Providers::elevenlabs_key` stays in the schema so #8 does not need another version bump.
- The wizard (#8) tests the key with the provider it will actually use, so it must read `active_provider` rather than assume Groq.
