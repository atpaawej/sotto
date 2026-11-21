# Typeless

An open-source desktop dictation app for Linux. You hold a key, speak, and clean text lands where your cursor is.

## Language

**Dictation**:
One recording, from the moment you start talking to the moment text lands in the app.
_Avoid_: session, take, capture

**Transcript**:
The exact words the speech engine heard, before any cleanup.
_Avoid_: raw text, STT output

**Recording**:
The audio captured during one dictation, kept on disk so it can be replayed or transcribed again.
_Avoid_: clip, audio file, take

**Cleanup**:
The fixed edits between transcript and insertion: filler removal, whitespace, spoken punctuation, and dictionary corrections.
_Avoid_: polish, AI edit, post-processing

**Dictionary**:
The user's own words and spelling corrections, applied literally during cleanup.
_Avoid_: vocabulary, glossary

**Injection**:
Putting the finished text into the app that had focus when the dictation started.
_Avoid_: paste, typing (both are ways to inject, not the act itself)

**Pill**:
The small always-on-top window that shows listening, working, and done.
_Avoid_: overlay, HUD, widget
