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

**Injector**:
The adapter that delivers finished text to the focused app. The clipboard path (write the clipboard, tap Ctrl+V) leads, and ydotool is the optional fallback. Settings names the active one and why it was chosen.
_Avoid_: paster

**Start at login**:
The app opens when you log in. The state lives in the user's autostart file.
_Avoid_: autostart

**Updates**:
Newer releases of the app, reported without installing. Installing is one click in Settings → About, or the installer script.
_Avoid_: upgrade

**Pill**:
The small always-on-top window that shows listening, working, and done.
_Avoid_: overlay, HUD, widget

**Provider**:
The speech service that transcribes a Recording with your key. Sotto keeps a key for each provider and sends audio only to the active one.
_Avoid_: engine, backend

**History**:
Every Dictation, newest first, with its Transcript, cleaned text, outcome, and Recording.
_Avoid_: log, archive, feed

**Failure**:
A typed reason a Dictation could not finish, with one short sentence for the Pill and the full detail for Settings.
_Avoid_: error message, exception, problem string

**Retry**:
Sending a stored Recording to the provider again, without asking the user to speak.
_Avoid_: redo, try again (the UI label may say Retry, the act is retry)
