# Cancelling is the pill's button, not a global Escape key

A dictation in flight can be cancelled, and cancelling discards the transcript instead of typing it. The trigger is a **Cancel** button on the pill while it is working. Escape is not captured.

## Why

The ticket asked for Escape. On Wayland a normal application cannot observe a key it does not own, and the only mechanism the portal offers for a global key is `GlobalShortcuts` — which gives the key to us exclusively. Binding Escape through it would take Escape away from every other application for as long as the binding exists, and GNOME asks the user to press the keys in a dialog for every bind, so binding it only while a transcription runs means a dialog in the middle of dictating.

Making the pill clickable for exactly that moment is the honest alternative: it appears where the user is already looking, it cannot fire while the user is typing in another app, and it costs no permission.

## Consequences

- The state machine has a Cancel input with no opinion about where it came from, so an Escape binding can be added later without touching the rules or their tests.
- The pill ignores the mouse in every phase except working, so it can never steal focus from the app the text is going into. `set_ignore_cursor_events` must be called after the window is shown — tao panics on a window GTK has not realised yet, and that panic kills the process.
- The ticket's Escape criterion stays open. Closing it properly means InputCapture (a screen-cast session plus an EIS connection), which is a different size of change than this ticket.
