# Groq is the provider that ships first

> Amended by #8: ElevenLabs Scribe is now wired behind the same `Transcriber`
> seam. The decision below still holds — Groq stays the default — but the
> "unwired" consequences no longer apply.

The first provider wired up is Groq's Whisper `large-v3-turbo`, not ElevenLabs Scribe. `active_provider` defaults to Groq, and a config written by the walking-skeleton build moves to Groq on load when it never held an ElevenLabs key. ElevenLabs was kept in the config schema so #8 could add its adapter without a version bump.

## Why

This was a scope call, not a verdict on quality: Groq was asked for, it needs one key and one multipart request, and its turbo model answers a ten-second recording in about 300 ms — measured, not assumed (413 ms from key release to text, all stages included). That leaves the latency budget free for the parts of the product that are actually hard: the gesture, the pill, injection.

Scribe v3 may still be the better transcriber. Nothing in the seam assumes otherwise: both are adapters behind `Transcriber`, and moving the default back is a config change plus one adapter.

## Consequences

- The migration only switches a file that had no ElevenLabs key. A user who pasted one keeps their choice, and their key is not deleted.
- `Providers::elevenlabs_key` stays in the schema so #8 did not need another version bump.
- The wizard (#8) tests the key with the provider it will actually use, so it reads `active_provider` rather than assuming Groq. The Speech section keeps both keys and can test either one.
- After #8, both adapters return typed failures through the same vocabulary. Selecting ElevenLabs no longer refuses the dictation; History can re-transcribe a Recording with the other provider and keeps both named results.
