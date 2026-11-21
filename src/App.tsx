import { useEffect, useState, type ReactNode } from "react";
import {
  keyLabel,
  providerLabel,
  readAppInfo,
  readConfig,
  type AppInfo,
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
      return (
        <section className="section">
          <h1 className="section-title">Speech</h1>
          <p className="section-deck">
            Dictation runs on a provider you bring a key for. ElevenLabs Scribe is
            the default; Groq is the cheaper second.
          </p>
          {snapshot && (
            <dl className="rows">
              <Row
                label="ElevenLabs"
                value={
                  <Stored stored={snapshot.config.providers.elevenlabs_key !== null} />
                }
                hint="Scribe v3."
              />
              <Row
                label="Groq"
                value={<Stored stored={snapshot.config.providers.groq_key !== null} />}
                hint="Whisper large v3 turbo."
              />
            </dl>
          )}
          {snapshot && (
            <p className="empty-note">
              {snapshot.config.providers.elevenlabs_key ||
              snapshot.config.providers.groq_key
                ? "Keys are stored in your settings file and sent only to the provider you choose."
                : "Sotto has nowhere to send your audio until a key is added. Keys go in your settings file and never anywhere else."}
            </p>
          )}
        </section>
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

function Stored({ stored }: { stored: boolean }) {
  return <>{stored ? "Key stored" : "No key yet"}</>;
}
