import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  cancelDictation,
  DICTATION_EVENT,
  readDictationState,
  type DictationState,
} from "./lib/api";

/**
 * The pill: what is happening, in one line at the bottom of the screen.
 *
 * It is a second webview (`index.html?window=pill`) in a window Rust positions.
 * While listening it ignores the mouse entirely, so a click where it sits goes
 * to the app underneath and never steals focus from where the text will land.
 */
export default function Pill() {
  const [dictation, setDictation] = useState<DictationState | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const since = useRef<number | null>(null);

  useEffect(() => {
    let cancelled = false;

    readDictationState().then(
      (state) => {
        if (!cancelled) setDictation(state);
      },
      () => {
        // The pill still works from events; a failed initial read is not fatal.
      },
    );

    const unlisten = listen<DictationState>(DICTATION_EVENT, (event) => {
      setDictation(event.payload);
    });

    return () => {
      cancelled = true;
      void unlisten.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    if (dictation?.phase !== "listening") {
      since.current = null;
      setElapsed(0);
      return;
    }

    since.current ??= Date.now();
    const tick = () => {
      if (since.current) setElapsed(Date.now() - since.current);
    };
    tick();
    const id = window.setInterval(tick, 100);
    return () => window.clearInterval(id);
  }, [dictation?.phase]);

  if (!dictation || dictation.phase === "idle") {
    return (
      <div className="pill pill-idle" aria-label="Sotto ready">
        <span className="pill-idle-dot" aria-hidden="true" />
        <span className="pill-idle-bar" aria-hidden="true" />
      </div>
    );
  }

  const { phase, locked, tapMode, message } = dictation;

  return (
    <div className={`pill pill-${phase}`} role="status" aria-live="polite">
      {phase === "listening" ? (
        <span className="pill-dot" aria-hidden="true" />
      ) : phase === "working" ? (
        <span className="pill-spinner" aria-hidden="true" />
      ) : phase === "done" ? (
        <span className="pill-check" aria-hidden="true">
          ✓
        </span>
      ) : (
        <span className="pill-mark" aria-hidden="true">
          ·
        </span>
      )}

      <span className="pill-label">{label(phase, locked, tapMode, message)}</span>

      {phase === "listening" && (
        <span className="pill-time" aria-hidden="true">
          {formatElapsed(elapsed)}
        </span>
      )}

      {phase === "working" && (
        <button
          className="pill-cancel"
          type="button"
          onClick={() => void cancelDictation()}
        >
          Cancel
        </button>
      )}
    </div>
  );
}

function label(
  phase: DictationState["phase"],
  locked: boolean,
  tapMode: boolean,
  message: string | null,
): string {
  switch (phase) {
    case "listening":
      if (locked) return "Listening — tap to finish";
      return tapMode ? "Listening — tap to finish" : "Listening";
    case "working":
      return "Transcribing";
    case "done":
      return "Inserted";
    default:
      return message ?? "Something went wrong";
  }
}

function formatElapsed(milliseconds: number): string {
  return `${(milliseconds / 1000).toFixed(1)} s`;
}
