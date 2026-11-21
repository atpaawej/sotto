import { useEffect, useState, type ReactNode } from "react";
import {
  isRecording,
  keyLabel,
  providerLabel,
  readAppInfo,
  readConfig,
  startDictation,
  stopDictation,
  type AppInfo,
  type Attempt,
  type ConfigSnapshot,
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

        <Section id={active} loaded={loaded} />
      </main>
    </div>
  );
}

function Section({ id, loaded }: { id: SectionId; loaded: Loaded }) {
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
                value={keyLabel(snapshot.config.trigger_key)}
                hint="Hold to talk. Double-tap to lock it on."
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
      return <SpeechSection snapshot={snapshot} />;

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

type Phase =
  | { kind: "idle" }
  | { kind: "starting" }
  | { kind: "recording"; startedAt: number }
  | { kind: "working" }
  | { kind: "done"; attempt: Attempt }
  | { kind: "failed"; message: string };

function SpeechSection({ snapshot }: { snapshot?: ConfigSnapshot }) {
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [elapsed, setElapsed] = useState(0);

  const hasKey = Boolean(snapshot?.config.providers.groq_key);
  const recording = phase.kind === "recording";
  const busy = phase.kind === "starting" || phase.kind === "working";

  // A reloaded window should still know a dictation is running.
  useEffect(() => {
    let cancelled = false;
    isRecording().then(
      (running) => {
        if (!cancelled && running) {
          setPhase({ kind: "recording", startedAt: Date.now() });
        }
      },
      () => {
        // The window can still start a dictation; this is a best-effort resync.
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (phase.kind !== "recording") return;
    const startedAt = phase.startedAt;
    const tick = () => setElapsed(Date.now() - startedAt);
    tick();
    const id = window.setInterval(tick, 100);
    return () => window.clearInterval(id);
  }, [phase]);

  async function toggle() {
    if (busy) return;

    if (phase.kind === "recording") {
      setPhase({ kind: "working" });
      try {
        const attempt = await stopDictation();
        setPhase({ kind: "done", attempt });
      } catch (error) {
        setPhase({ kind: "failed", message: String(error) });
      }
      return;
    }

    setPhase({ kind: "starting" });
    try {
      await startDictation();
      setPhase({ kind: "recording", startedAt: Date.now() });
    } catch (error) {
      setPhase({ kind: "failed", message: String(error) });
    }
  }

  const label = recording
    ? "Stop and transcribe"
    : phase.kind === "working"
      ? "Transcribing…"
      : phase.kind === "starting"
        ? "Opening the microphone…"
        : "Start dictation";

  return (
    <section className="section">
      <h1 className="section-title">Speech</h1>
      <p className="section-deck">
        Groq transcribes every dictation with Whisper large v3 turbo, using the
        key in your settings file. The hotkey arrives in a later build, so this
        button is how you dictate for now.
      </p>

      <div className="dictation">
        <button
          className={recording ? "button button-live" : "button"}
          type="button"
          onClick={toggle}
          disabled={!hasKey || busy}
        >
          {recording && <span className="recording-dot" aria-hidden="true" />}
          {label}
        </button>
        {recording && (
          <span className="timer" role="timer" aria-live="off">
            {formatElapsed(elapsed)}
          </span>
        )}
      </div>

      <p className="dictation-hint" aria-live="polite">
        {!hasKey ? (
          <>
            Add a Groq key to <span className="mono">{snapshot?.path}</span> to
            dictate. Keys never leave this machine except to the provider.
          </>
        ) : recording ? (
          "Recording. Press stop when you have finished."
        ) : (
          "Speak naturally. The audio is deleted as soon as the text comes back."
        )}
      </p>

      {phase.kind === "failed" && (
        <div className="notice" role="alert">
          <p className="notice-title">That dictation did not go through.</p>
          <p className="notice-detail">{phase.message}</p>
        </div>
      )}

      {phase.kind === "done" && <Result attempt={phase.attempt} />}
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
