# Hold-to-talk, until the desktop proves it only sends presses

The session starts in hold mode: a press opens the microphone and a release finishes the dictation. If a second press arrives while the first is still recording, releases are evidently not being delivered, so the session switches to tap mode for the rest of the run — one press starts, the next stops — and says so in the window.

## Why

`GlobalShortcuts` promises `Activated` and `Deactivated`, and GNOME 50.1 delivers both. Not every backend does: some only send a press, and the portal spec does not require the pair. Asking the portal at bind time whether it supports releases gives no answer, so the only reliable evidence is behaviour. Guessing at startup would either break hold-to-talk everywhere or leave tap users with a dictation that never ends.

The second press is unambiguous: in hold mode there is no reason for a press while the key is still held, and the user's next press after letting go is exactly the evidence needed.

## Consequences

- One dictation is finished by the press that reveals tap mode, so the first attempt on such a desktop produces text from a press that felt like a start. The alternative — discarding it — throws away speech the user already gave.
- The mode is remembered for the process, shown in the window, and tested with fake ports; it is not written to config, because it describes the desktop, not a preference.
- A desktop that sends a spurious second press for some other reason would be misread as tap mode. Nothing in the protocol distinguishes the two, and the switch is visible, so this is preferred to a mode the user cannot start.
