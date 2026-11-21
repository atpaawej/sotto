import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  cancelDictation,
  DICTATION_EVENT,
  keyLabel,
  providerLabel,
  readAccess,
  readAppInfo,
  readConfig,
  readDictationState,
  retryKeyboardAccess,
  toggleDictation,
  type Access,
  type AppInfo,
  type Attempt,
  type ConfigSnapshot,
  type DictationState,
} from "./lib/api";

type Loaded =
  | { status: "loading" }
  | { status: "ready"; snapshot: ConfigSnapshot; info: AppInfo }
  | { status: "failed"; message: string };

const SECTIONS = [
  { id: "general", label: "General" },
  { id: "speech", label: "Speech" },
  { id: "cleanup", label: "Cleanup" },
  { id: "dictionary", label: "Dictionary" },
  { id: "history", label: "History" },
  { id: "about", label: "About" },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

export default function App() {
  const [loaded, setLoaded] = useState<Loaded>({ status: "loading" });
  const [active, setActive] = useState<SectionId>("general");
  const [access, setAccess] = useState<Access | null>(null);

  const refreshAccess = useCallback(() => {
    readAccess().then(setAccess, () => {
      // The window can still work without the status; it only loses detail.
    });
  }, []);

  useEffect(() => {
    refreshAccess();
  }, [refreshAccess]);

  // The portal answers its dialogs late, so poll until both capabilities are
  // settled rather than leaving the window showing a stale "not bound".
  useEffect(() => {
    if (!access) return;
    if (access.hotkey.bound && access.injection.ready) return;
    const id = window.setInterval(refreshAccess, 2_000);
    return () => window.clearInterval(id);
  }, [access, refreshAccess]);
  useEffect(() => {
    let cancelled = false;

    Promise.all([readConfig(), readAppInfo()]).then(
      ([snapshot, info]) => {
        if (!cancelled) setLoaded({ status: "ready", snapshot, info });
      },
      (error: unknown) => {
        if (!cancelled) setLoaded({ status: "failed", message: String(error) });
      },
    );

    return () => {
      cancelled = true;
    };
  }, []);

  const version = loaded.status === "ready" ? loaded.info.version : "—";

  return (
    <div className="app">
      <nav className="sidebar" aria-label="Settings">
        <p className="wordmark">Sotto</p>
        <ul className="nav">
          {SECTIONS.map((section) => (
            <li key={section.id}>
              <button
                className="nav-button"
                type="button"
                aria-current={active === section.id ? "page" : undefined}
                onClick={() => setActive(section.id)}
              >
                {section.label}
              </button>
            </li>
          ))}
        </ul>
        <p className="sidebar-foot">Version {version}</p>
      </nav>

      <main className="content">
        {loaded.status === "failed" && (
          <div className="notice" role="alert">
            <p className="notice-title">Your settings could not be read.</p>
            <p className="notice-detail">
              The file was left untouched. Fix it by hand, then reopen Sotto.
              <br />
              <span className="mono">{loaded.message}</span>
            </p>
          </div>
        )}

        <Section
          id={active}
          loaded={loaded}
          access={access}
          refreshAccess={refreshAccess}
        />
      </main>
    </div>
  );
}

function Section({
  id,
  loaded,
  access,
  refreshAccess,
}: {
  id: SectionId;
  loaded: Loaded;
  access: Access | null;
  refreshAccess: () => void;
}) {
  const snapshot = loaded.status === "ready" ? loaded.snapshot : undefined;
  const info = loaded.status === "ready" ? loaded.info : undefined;

  switch (id) {
    case "general":
      return (
        <section className="section">
          <h1 className="section-title">General</h1>
          <p className="section-deck">
            How dictation starts, and where the rest of these settings live.
          </p>
          {snapshot ? (
            <dl className="rows">
              <Row
                label="Trigger key"
                value={
                  access?.hotkey.trigger ?? keyLabel(snapshot.config.trigger_key)
                }
                hint={
                  access?.hotkey.bound
                    ? "Hold to dictate anywhere. Double-tap to lock it on."
                    : "Not bound yet — Speech says what went wrong."
                }
              />
              <Row
                label="Provider"
                value={providerLabel(snapshot.config.active_provider)}
                hint="Only the provider you pick receives your audio."
              />
              <Row
                label="Settings file"
                value={<span className="mono">{snapshot.path}</span>}
                hint={
                  snapshot.mode === "600"
                    ? "Mode 600. Only your user account can read it."
                    : `Mode ${snapshot.mode}.`
                }
              />
            </dl>
          ) : (
            <p className="quiet">Settings are not available.</p>
          )}
        </section>
      );

    case "speech":
      return (
        <SpeechSection
          snapshot={snapshot}
          access={access}
          refreshAccess={refreshAccess}
        />
      );

    case "cleanup":
      return (
        <section className="section">
          <h1 className="section-title">Cleanup</h1>
          <p className="section-deck">
            Four rules run over every transcript before it is typed. None of them
            use a language model, so the text can only lose words you did not mean
            to say.
          </p>
          {snapshot && (
            <dl className="rows">
              <Rule label="Fillers" on={snapshot.config.cleanup.fillers} />
              <Rule
                label="Spoken punctuation"
                on={snapshot.config.cleanup.spoken_punctuation}
              />
              <Rule label="Whitespace" on={snapshot.config.cleanup.whitespace} />
              <Rule label="Dictionary" on={snapshot.config.cleanup.dictionary} />
            </dl>
          )}
          <p className="empty-note">
            These states are saved now, but the rulebook is not wired up in this
            build yet.
          </p>
        </section>
      );

    case "dictionary":
      return (
        <section className="section">
          <h1 className="section-title">Dictionary</h1>
          <p className="section-deck">
            Names and jargon you want spelled your way. Entries are matched whole
            and case-insensitively, and never leave your machine.
          </p>
          <div className="empty">
            {snapshot && snapshot.config.dictionary.length > 0 ? (
              <p className="empty-line">
                {snapshot.config.dictionary.length} words saved.
              </p>
            ) : (
              <p className="empty-line">No words yet.</p>
            )}
            <p className="empty-note">
              Editing words is not wired up in this build yet.
            </p>
          </div>
        </section>
      );

    case "history":
      return (
        <section className="section">
          <h1 className="section-title">History</h1>
          <p className="section-deck">
            Every dictation, with its transcript and its audio, kept on this
            machine and nowhere else.
          </p>
          <div className="empty">
            <p className="empty-line">Nothing dictated yet.</p>
            <p className="empty-note">
              Your first dictation will appear here, with the audio to replay.
            </p>
          </div>
        </section>
      );

    case "about":
      return (
        <section className="section">
          <h1 className="section-title">About</h1>
          <p className="section-deck">
            Sotto is open source, and the only computer it talks to is the speech
            provider you chose.
          </p>
          {info && (
            <dl className="rows">
              <Row label="Version" value={<span>{info.version}</span>} />
              <Row label="License" value={info.license} />
              <Row label="Source" value={<span className="mono">{info.repository}</span>} />
            </dl>
          )}
        </section>
      );
  }
}

function Row({
  label,
  value,
  hint,
}: {
  label: string;
  value: ReactNode;
  hint?: string;
}) {
  return (
    <div className="row">
      <dt className="row-label">{label}</dt>
      <dd className="row-value">
        <p>{value}</p>
        {hint && <p className="row-hint">{hint}</p>}
      </dd>
    </div>
  );
}

function Rule({ label, on }: { label: string; on: boolean }) {
  return <Row label={label} value={on ? "On" : "Off"} />;
}

function SpeechSection({
  snapshot,
  access,
  refreshAccess,
}: {
  snapshot?: ConfigSnapshot;
  access: Access | null;
  refreshAccess: () => void;
}) {
  const [dictation, setDictation] = useState<DictationState | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [busy, setBusy] = useState(false);
  const [retrying, setRetrying] = useState(false);
  const since = useRef<number | null>(null);

  const hasKey = Boolean(snapshot?.config.providers.groq_key);
  const phase = dictation?.phase ?? "idle";
  const recording = phase === "listening";
  const working = phase === "working";

  // The hotkey can start a dictation while this window is closed, so the state
  // comes from the backend rather than from clicks here.
  useEffect(() => {
    let cancelled = false;

    readDictationState().then(
      (state) => {
        if (!cancelled) setDictation(state);
      },
      () => {
        // Events still arrive; only the initial resync is lost.
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
    if (phase !== "listening") {
      since.current = null;
      setElapsed(0);
      return;
    }

    // A dictation started by the hotkey arrives as an event, so the clock
    // starts when this window hears about it.
    since.current ??= Date.now();
    const tick = () => {
      if (since.current) setElapsed(Date.now() - since.current);
    };
    tick();
    const id = window.setInterval(tick, 100);
    return () => window.clearInterval(id);
  }, [phase]);

  async function toggle() {
    if (busy || working) return;
    setBusy(true);
    try {
      await toggleDictation();
    } catch (error) {
      setDictation({
        phase: "failed",
        locked: false,
        tapMode: false,
        message: String(error),
        attempt: null,
      });
    } finally {
      setBusy(false);
    }
  }

  async function cancel() {
    try {
      await cancelDictation();
    } catch (error) {
      setDictation({
        phase: "failed",
        locked: false,
        tapMode: false,
        message: String(error),
        attempt: null,
      });
    }
  }

  async function retryAccess() {
    setRetrying(true);
    try {
      await retryKeyboardAccess();
      window.setTimeout(refreshAccess, 800);
    } catch (error) {
      setDictation({
        phase: "failed",
        locked: false,
        tapMode: false,
        message: String(error),
        attempt: null,
      });
    } finally {
      setRetrying(false);
    }
  }

  const label = recording
    ? "Stop and insert"
    : working
      ? "Transcribing…"
      : "Start dictation";

  const trigger = access?.hotkey.trigger;

  return (
    <section className="section">
      <h1 className="section-title">Speech</h1>
      <p className="section-deck">
        {trigger
          ? `Hold ${trigger} and speak; release and the words land where your cursor is. The audio goes to Groq with your key and nowhere else.`
          : "Hold the shortcut and speak; the words land where your cursor is. The audio goes to Groq with your key and nowhere else."}
      </p>

      {access?.hotkey.problem && (
        <div className="notice" role="alert">
          <p className="notice-title">The shortcut is not bound.</p>
          <p className="notice-detail">{access.hotkey.problem}</p>
        </div>
      )}

      {access?.injection.problem && (
        <div className="notice" role="alert">
          <p className="notice-title">Text cannot be typed yet.</p>
          <p className="notice-detail">{access.injection.problem}</p>
          <button
            className="button button-quiet"
            type="button"
            onClick={() => void retryAccess()}
            disabled={retrying}
          >
            {retrying ? "Asking…" : "Ask again"}
          </button>
        </div>
      )}

      <div className="dictation">
        <button
          className={recording ? "button button-live" : "button"}
          type="button"
          onClick={() => void toggle()}
          disabled={!hasKey || working || busy}
        >
          {recording && <span className="recording-dot" aria-hidden="true" />}
          {label}
        </button>

        {recording && (
          <span className="timer" role="timer" aria-live="off">
            {formatElapsed(elapsed)}
          </span>
        )}

        {working && (
          <button
            className="button button-quiet"
            type="button"
            onClick={() => void cancel()}
          >
            Cancel
          </button>
        )}
      </div>

      <p className="dictation-hint" aria-live="polite">
        {!hasKey ? (
          <>
            Add a Groq key to <span className="mono">{snapshot?.path}</span> to
            dictate. Keys never leave this machine except to the provider.
          </>
        ) : recording ? (
          dictation?.tapMode || dictation?.locked
            ? "Recording. Tap the shortcut again when you have finished."
            : "Recording. Release the shortcut when you have finished."
        ) : working ? (
          "The recording is on its way. Cancel keeps it out of your document."
        ) : phase === "hint" && dictation?.message ? (
          dictation.message
        ) : trigger ? (
          `Hold ${trigger} anywhere. Double-tap to lock it on until the next tap.`
        ) : (
          "Speak naturally. The audio is deleted as soon as the text comes back."
        )}
      </p>

      {phase === "failed" && dictation?.message && (
        <div className="notice" role="alert">
          <p className="notice-title">That dictation did not go through.</p>
          <p className="notice-detail">{dictation.message}</p>
        </div>
      )}

      {phase === "done" && dictation?.attempt && (
        <Result attempt={dictation.attempt} />
      )}
    </section>
  );
}

function Result({ attempt }: { attempt: Attempt }) {
  const { timings } = attempt;
  const recorded = formatElapsed(attempt.durationMs);

  return (
    <div className="result">
      <blockquote className="transcript">
        {attempt.transcript || "No words came back. Try speaking a little closer."}
      </blockquote>
      <p className="result-meta">
        {attempt.provider === "groq" ? "Groq" : attempt.provider}{" "}
        <span className="mono">{attempt.model}</span>, {recorded} of audio, back
        in {formatMilliseconds(timings.totalMs)}.
      </p>
      <p className="timings">
        Stopping {formatMilliseconds(timings.stopMs)}, encoding{" "}
        {formatMilliseconds(timings.encodeMs)}, upload{" "}
        {formatMilliseconds(timings.uploadMs)}, provider{" "}
        {formatMilliseconds(timings.providerMs)}.
      </p>
    </div>
  );
}

/** 3400 → "3.4 s", 12400 → "12.4 s" */
function formatElapsed(milliseconds: number): string {
  return `${(milliseconds / 1000).toFixed(1)} s`;
}

function formatMilliseconds(milliseconds: number): string {
  return milliseconds < 1000
    ? `${Math.round(milliseconds)} ms`
    : `${(milliseconds / 1000).toFixed(2)} s`;
}
