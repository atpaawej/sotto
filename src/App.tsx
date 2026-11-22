import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  asFailure,
  cancelDictation,
  copyHistory,
  deleteAllHistory,
  deleteHistory,
  DICTATION_EVENT,
  keyLabel,
  otherProvider,
  providerLabel,
  PROVIDERS,
  readAccess,
  readAppInfo,
  readConfig,
  readDictationState,
  readHistory,
  readRecording,
  retryDictation,
  retryHistory,
  retryHistoryOther,
  retryKeyboardAccess,
  testProvider,
  toggleDictation,
  updateCleanup,
  updateDictionary,
  updateProvider,
  updateProviderKey,
  updateRetention,
  type Access,
  type AppInfo,
  type Attempt,
  type Cleanup,
  type CleanupLevel,
  type ConfigSnapshot,
  type DictationState,
  type Failure,
  type HistoryEntry,
  type ProviderId,
} from "./lib/api";
import Wizard from "./Wizard";

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
  // Decided on the first successful load: saving a key mid-wizard must not
  // close it before the live test and the test dictation have both passed.
  const [wizard, setWizard] = useState<ConfigSnapshot | null | undefined>(
    undefined,
  );

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

  useEffect(() => {
    if (wizard !== undefined || loaded.status !== "ready") return;
    const { providers } = loaded.snapshot.config;
    const hasKey = [providers.groq_key, providers.elevenlabs_key].some((key) =>
      Boolean(key?.trim()),
    );
    setWizard(hasKey ? null : loaded.snapshot);
  }, [loaded, wizard]);

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
        {wizard ? (
          <Wizard
            initial={wizard}
            access={access}
            onSnapshot={(snapshot) =>
              setLoaded((prev) =>
                prev.status === "ready" ? { ...prev, snapshot } : prev,
              )
            }
            onDone={() => setWizard(null)}
          />
        ) : (
          <>
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
              onSnapshot={(snapshot) =>
                setLoaded((prev) =>
                  prev.status === "ready" ? { ...prev, snapshot } : prev,
                )
              }
            />
          </>
        )}
      </main>
    </div>
  );
}

function Section({
  id,
  loaded,
  access,
  refreshAccess,
  onSnapshot,
}: {
  id: SectionId;
  loaded: Loaded;
  access: Access | null;
  refreshAccess: () => void;
  onSnapshot: (snapshot: ConfigSnapshot) => void;
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
          onSnapshot={onSnapshot}
        />
      );

    case "cleanup":
      return (
        <CleanupSection snapshot={snapshot} onSnapshot={onSnapshot} />
      );

    case "dictionary":
      return (
        <DictionarySection snapshot={snapshot} onSnapshot={onSnapshot} />
      );

    case "history":
      return <HistorySection snapshot={snapshot} onSnapshot={onSnapshot} />;

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

const CLEANUP_LEVELS: {
  id: CleanupLevel;
  label: string;
  hint: string;
}[] = [
  {
    id: "none",
    label: "None",
    hint: "Skips the cleanup call and inserts the raw Whisper transcript.",
  },
  {
    id: "light",
    label: "Light",
    hint: "Strips fillers, fixes punctuation, resolves self-corrections, and keeps your exact wording.",
  },
  {
    id: "medium",
    label: "Medium",
    hint: "Light cleanup plus light rewording for clarity and conciseness.",
  },
];

function CleanupSection({
  snapshot,
  onSnapshot,
}: {
  snapshot?: ConfigSnapshot;
  onSnapshot: (snapshot: ConfigSnapshot) => void;
}) {
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const cleanup = snapshot?.config.cleanup;

  async function save(next: Cleanup) {
    setSaving(true);
    setError(null);
    try {
      const updated = await updateCleanup(next);
      onSnapshot(updated);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  function setLevel(level: CleanupLevel) {
    if (!cleanup) return;
    void save({ ...cleanup, level });
  }

  function toggleRule(
    key: "fillers" | "spoken_punctuation" | "whitespace" | "dictionary",
  ) {
    if (!cleanup) return;
    void save({ ...cleanup, [key]: !cleanup[key] });
  }

  const activeLevel = CLEANUP_LEVELS.find((item) => item.id === cleanup?.level);

  return (
    <section className="section">
      <h1 className="section-title">Cleanup</h1>
      <p className="section-deck">
        A fast second pass on Groq cleans the raw transcript before it lands at
        your cursor. If cleanup ever fails, the raw transcript is inserted and
        the error is shown.
      </p>

      {error && (
        <div className="notice" role="alert">
          <p className="notice-title">Could not save cleanup settings.</p>
          <p className="notice-detail">{error}</p>
        </div>
      )}

      {cleanup ? (
        <dl className="rows">
          <div className="row">
            <dt className="row-label">Level</dt>
            <dd className="row-value">
              <div className="segmented" role="group" aria-label="Cleanup level">
                {CLEANUP_LEVELS.map((item) => (
                  <button
                    key={item.id}
                    type="button"
                    className="segmented-button"
                    aria-pressed={cleanup.level === item.id}
                    disabled={saving}
                    onClick={() => setLevel(item.id)}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
              {activeLevel && <p className="row-hint">{activeLevel.hint}</p>}
            </dd>
          </div>

          <Row
            label="Model"
            value={<span className="mono">{cleanup.model}</span>}
            hint="Uses your Groq key with reasoning disabled for low latency."
          />

          <RuleToggle
            label="Fillers"
            on={cleanup.fillers}
            disabled={saving || cleanup.level === "none"}
            onToggle={() => toggleRule("fillers")}
          />
          <RuleToggle
            label="Spoken punctuation"
            on={cleanup.spoken_punctuation}
            disabled={saving || cleanup.level === "none"}
            onToggle={() => toggleRule("spoken_punctuation")}
          />
          <RuleToggle
            label="Whitespace"
            on={cleanup.whitespace}
            disabled={saving || cleanup.level === "none"}
            onToggle={() => toggleRule("whitespace")}
          />
          <RuleToggle
            label="Dictionary"
            on={cleanup.dictionary}
            disabled={saving}
            onToggle={() => toggleRule("dictionary")}
          />
        </dl>
      ) : (
        <p className="quiet">Settings are not available.</p>
      )}
    </section>
  );
}

function RuleToggle({
  label,
  on,
  disabled,
  onToggle,
}: {
  label: string;
  on: boolean;
  disabled: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="row">
      <dt className="row-label">{label}</dt>
      <dd className="row-value">
        <button
          type="button"
          className="button button-quiet toggle-button"
          aria-pressed={on}
          disabled={disabled}
          onClick={onToggle}
        >
          {on ? "On" : "Off"}
        </button>
      </dd>
    </div>
  );
}

function DictionarySection({
  snapshot,
  onSnapshot,
}: {
  snapshot?: ConfigSnapshot;
  onSnapshot: (snapshot: ConfigSnapshot) => void;
}) {
  const [draft, setDraft] = useState("");
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [editValue, setEditValue] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const entries = snapshot?.config.dictionary ?? [];

  async function save(next: string[]) {
    setSaving(true);
    setError(null);
    try {
      const updated = await updateDictionary(next);
      onSnapshot(updated);
      return true;
    } catch (err) {
      setError(String(err));
      return false;
    } finally {
      setSaving(false);
    }
  }

  async function addEntry(event: React.FormEvent) {
    event.preventDefault();
    const trimmed = draft.trim();
    if (!trimmed) return;
    const ok = await save([...entries, trimmed]);
    if (ok) setDraft("");
  }

  async function saveEdit(index: number) {
    const trimmed = editValue.trim();
    if (!trimmed) return;
    const next = entries.map((item, i) => (i === index ? trimmed : item));
    const ok = await save(next);
    if (ok) {
      setEditingIndex(null);
      setEditValue("");
    }
  }

  async function deleteEntry(index: number) {
    const next = entries.filter((_, i) => i !== index);
    await save(next);
    if (editingIndex === index) {
      setEditingIndex(null);
      setEditValue("");
    }
  }

  return (
    <section className="section">
      <h1 className="section-title">Dictionary</h1>
      <p className="section-deck">
        Names, jargon, and product terms you want spelled your way. Sent as a
        hint to Whisper and applied contextually by Cleanup.
      </p>

      {error && (
        <div className="notice" role="alert">
          <p className="notice-title">Could not save dictionary.</p>
          <p className="notice-detail">{error}</p>
        </div>
      )}

      {snapshot ? (
        <>
          <form className="dict-form" onSubmit={(e) => void addEntry(e)}>
            <input
              type="text"
              className="text-input"
              placeholder="Add a word or phrase (e.g. Sotto, Wayland)"
              value={draft}
              disabled={saving}
              onChange={(e) => setDraft(e.target.value)}
            />
            <button
              type="submit"
              className="button"
              disabled={saving || !draft.trim()}
            >
              Add
            </button>
          </form>

          {entries.length > 0 ? (
            <ul className="dict-list">
              {entries.map((entry, index) => (
                <li key={`${entry}-${index}`} className="dict-item">
                  {editingIndex === index ? (
                    <div className="dict-edit">
                      <input
                        type="text"
                        className="text-input"
                        value={editValue}
                        disabled={saving}
                        onChange={(e) => setEditValue(e.target.value)}
                      />
                      <button
                        type="button"
                        className="button"
                        disabled={saving || !editValue.trim()}
                        onClick={() => void saveEdit(index)}
                      >
                        Save
                      </button>
                      <button
                        type="button"
                        className="button button-quiet"
                        disabled={saving}
                        onClick={() => {
                          setEditingIndex(null);
                          setEditValue("");
                        }}
                      >
                        Cancel
                      </button>
                    </div>
                  ) : (
                    <>
                      <span className="dict-term">{entry}</span>
                      <div className="dict-actions">
                        <button
                          type="button"
                          className="button button-quiet"
                          disabled={saving}
                          onClick={() => {
                            setEditingIndex(index);
                            setEditValue(entry);
                          }}
                        >
                          Edit
                        </button>
                        <button
                          type="button"
                          className="button button-quiet"
                          disabled={saving}
                          onClick={() => void deleteEntry(index)}
                        >
                          Delete
                        </button>
                      </div>
                    </>
                  )}
                </li>
              ))}
            </ul>
          ) : (
            <div className="empty">
              <p className="empty-line">No words yet.</p>
              <p className="empty-note">
                Add names or technical terms above to keep their spelling
                consistent.
              </p>
            </div>
          )}
        </>
      ) : (
        <p className="quiet">Settings are not available.</p>
      )}
    </section>
  );
}

const RETENTION_CHOICES: { value: string; label: string }[] = [
  { value: "", label: "Keep everything" },
  { value: "7", label: "7 days" },
  { value: "30", label: "30 days" },
  { value: "90", label: "90 days" },
  { value: "365", label: "1 year" },
];

function HistorySection({
  snapshot,
  onSnapshot,
}: {
  snapshot?: ConfigSnapshot;
  onSnapshot: (snapshot: ConfigSnapshot) => void;
}) {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [playing, setPlaying] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [deletingAll, setDeletingAll] = useState(false);
  const [savingRetention, setSavingRetention] = useState(false);
  const [reload, setReload] = useState(0);
  const audio = useRef<HTMLAudioElement | null>(null);
  const audioUrl = useRef<string | null>(null);
  const copiedTimer = useRef<number | null>(null);

  const stopPlayback = useCallback(() => {
    audio.current?.pause();
    audio.current = null;
    if (audioUrl.current) {
      URL.revokeObjectURL(audioUrl.current);
      audioUrl.current = null;
    }
    setPlaying(null);
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);

    // Typing should not send a query per keystroke.
    const timer = window.setTimeout(
      () => {
        readHistory(query).then(
          (next) => {
            if (cancelled) return;
            setEntries(next);
            setError(null);
            setLoading(false);
          },
          (reason: unknown) => {
            if (cancelled) return;
            setError(String(reason));
            setLoading(false);
          },
        );
      },
      query ? 160 : 0,
    );

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query, reload]);

  useEffect(
    () => () => {
      audio.current?.pause();
      if (audioUrl.current) URL.revokeObjectURL(audioUrl.current);
      if (copiedTimer.current) window.clearTimeout(copiedTimer.current);
    },
    [],
  );

  async function copy(entry: HistoryEntry) {
    try {
      await copyHistory(entry.id);
      setCopied(entry.id);
      if (copiedTimer.current) window.clearTimeout(copiedTimer.current);
      copiedTimer.current = window.setTimeout(() => setCopied(null), 1_600);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function play(entry: HistoryEntry) {
    if (playing === entry.id) {
      stopPlayback();
      return;
    }
    stopPlayback();
    try {
      const bytes = await readRecording(entry.id);
      const url = URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
      const element = new Audio(url);
      audio.current = element;
      audioUrl.current = url;
      element.onended = stopPlayback;
      element.onerror = () => {
        stopPlayback();
        setError("That recording could not be played.");
      };
      setPlaying(entry.id);
      await element.play();
    } catch (reason) {
      stopPlayback();
      setError(String(reason));
    }
  }

  async function remove(entry: HistoryEntry) {
    if (playing === entry.id) stopPlayback();
    try {
      await deleteHistory(entry.id);
      setReload((count) => count + 1);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function removeAll() {
    if (
      !window.confirm(
        "Delete every dictation and its audio? This cannot be undone.",
      )
    ) {
      return;
    }
    setDeletingAll(true);
    stopPlayback();
    try {
      await deleteAllHistory();
      setReload((count) => count + 1);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setDeletingAll(false);
    }
  }

  async function retry(entry: HistoryEntry) {
    try {
      await retryHistory(entry.id);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function retryOther(entry: HistoryEntry) {
    try {
      await retryHistoryOther(entry.id);
      setReload((count) => count + 1);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function changeRetention(value: string) {
    setSavingRetention(true);
    try {
      const updated = await updateRetention(value === "" ? null : Number(value));
      onSnapshot(updated);
      setReload((count) => count + 1);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSavingRetention(false);
    }
  }

  const retention =
    snapshot?.config.retention_days === null ||
    snapshot?.config.retention_days === undefined
      ? ""
      : String(snapshot.config.retention_days);

  return (
    <section className="section section-wide">
      <h1 className="section-title">History</h1>
      <p className="section-deck">
        Every dictation, with its transcript and its audio, kept on this machine
        and nowhere else. A failed dictation keeps its recording so Retry can
        send it again without asking you to speak. Retrying with the other
        provider keeps both results, each named with the provider that heard it.
      </p>

      {error && (
        <div className="notice" role="alert">
          <p className="notice-title">History had a problem.</p>
          <p className="notice-detail">{error}</p>
        </div>
      )}

      <div className="history-toolbar">
        <input
          type="search"
          className="text-input history-search"
          placeholder="Search transcripts, providers, models"
          aria-label="Search history"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <label className="history-retention">
          <span className="history-retention-label">Keep</span>
          <select
            className="text-input history-select"
            value={retention}
            disabled={savingRetention || !snapshot}
            onChange={(event) => void changeRetention(event.target.value)}
          >
            {RETENTION_CHOICES.map((choice) => (
              <option key={choice.value} value={choice.value}>
                {choice.label}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          className="button button-quiet history-delete-all"
          onClick={() => void removeAll()}
          disabled={deletingAll || entries.length === 0}
        >
          {deletingAll ? "Deleting…" : "Delete all"}
        </button>
      </div>

      {loading && entries.length === 0 ? (
        <p className="quiet history-status">Reading history…</p>
      ) : entries.length === 0 ? (
        <div className="empty">
          <p className="empty-line">
            {query ? "No dictations match that search." : "Nothing dictated yet."}
          </p>
          <p className="empty-note">
            {query
              ? "Search looks at the transcript, the cleaned text, the provider and the model."
              : "Your first dictation will appear here, with the audio to replay."}
          </p>
        </div>
      ) : (
        <ol className="history-list">
          {entries.map((entry) => {
            const text = entry.cleaned.trim() || entry.transcript.trim();
            return (
              <li key={entry.id} className="history-item">
                <div className="history-head">
                  <time className="history-when" dateTime={new Date(entry.createdAtMs).toISOString()}>
                    {formatWhen(entry.createdAtMs)}
                  </time>
                  <span className="history-facts">
                    {formatElapsed(entry.durationMs)} ·{" "}
                    {providerLabel(entry.provider)}{" "}
                    <span className="mono">{entry.model}</span>
                  </span>
                </div>

                <p className={text ? "history-text" : "history-text history-text-empty"}>
                  {text || "No words came back from this recording."}
                </p>

                {entry.failure && (
                  <p className="history-failure">
                    <span className="history-failure-reason">
                      {entry.failure.reason}
                    </span>{" "}
                    <span className="history-failure-detail">
                      {entry.failure.detail}
                    </span>
                  </p>
                )}

                <div className="history-actions">
                  <button
                    type="button"
                    className="button button-quiet"
                    onClick={() => void play(entry)}
                    disabled={!entry.recordingPath}
                  >
                    {playing === entry.id ? "Stop" : "Play"}
                  </button>
                  <button
                    type="button"
                    className="button button-quiet"
                    onClick={() => void copy(entry)}
                    disabled={!text}
                  >
                    {copied === entry.id ? "Copied" : "Copy"}
                  </button>
                  {entry.failure &&
                    entry.injection !== "inserted" &&
                    entry.recordingPath && (
                    <button
                      type="button"
                      className="button button-quiet"
                      onClick={() => void retry(entry)}
                    >
                      Retry
                    </button>
                  )}
                  {(entry.provider === "groq" || entry.provider === "eleven_labs") &&
                    entry.recordingPath && (
                    <button
                      type="button"
                      className="button button-quiet"
                      onClick={() => void retryOther(entry)}
                    >
                      Retry with {providerLabel(otherProvider(entry.provider))}
                    </button>
                  )}
                  <button
                    type="button"
                    className="button button-quiet history-remove"
                    onClick={() => void remove(entry)}
                  >
                    Delete
                  </button>
                </div>
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}

function formatWhen(milliseconds: number): string {
  const date = new Date(milliseconds);
  const sameYear = date.getFullYear() === new Date().getFullYear();
  return new Intl.DateTimeFormat(undefined, {
    day: "numeric",
    month: "short",
    ...(sameYear ? {} : { year: "numeric" }),
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

type KeyTest = { status: "testing" | "ok" | "failed"; failure?: Failure };

function SpeechSection({
  snapshot,
  access,
  refreshAccess,
  onSnapshot,
}: {
  snapshot?: ConfigSnapshot;
  access: Access | null;
  refreshAccess: () => void;
  onSnapshot: (snapshot: ConfigSnapshot) => void;
}) {
  const [dictation, setDictation] = useState<DictationState | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [busy, setBusy] = useState(false);
  const [retrying, setRetrying] = useState(false);
  const [retryingDictation, setRetryingDictation] = useState(false);
  const [keyDrafts, setKeyDrafts] = useState<Partial<Record<ProviderId, string>>>({});
  const [keyTests, setKeyTests] = useState<Partial<Record<ProviderId, KeyTest>>>({});
  const [savingKey, setSavingKey] = useState<ProviderId | null>(null);
  const [switching, setSwitching] = useState(false);
  const [providerError, setProviderError] = useState<string | null>(null);
  const since = useRef<number | null>(null);

  const activeProvider = snapshot?.config.active_provider ?? "groq";
  const activeKey =
    activeProvider === "groq"
      ? snapshot?.config.providers.groq_key
      : snapshot?.config.providers.elevenlabs_key;
  const hasKey = Boolean(activeKey?.trim());
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
        failure: null,
        retryId: null,
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
        failure: null,
        retryId: null,
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
        failure: null,
        retryId: null,
      });
    } finally {
      setRetrying(false);
    }
  }

  async function retryLast() {
    setRetryingDictation(true);
    try {
      await retryDictation();
    } catch (error) {
      setDictation({
        phase: "failed",
        locked: false,
        tapMode: false,
        message: String(error),
        attempt: null,
        failure: null,
        retryId: null,
      });
    } finally {
      setRetryingDictation(false);
    }
  }

  async function chooseProvider(next: ProviderId) {
    setSwitching(true);
    setProviderError(null);
    try {
      onSnapshot(await updateProvider(next));
    } catch (error) {
      setProviderError(asFailure(error).detail);
    } finally {
      setSwitching(false);
    }
  }

  async function saveKey(provider: ProviderId) {
    const draft = keyDrafts[provider]?.trim();
    if (!draft) return;
    setSavingKey(provider);
    setProviderError(null);
    try {
      onSnapshot(await updateProviderKey(provider, draft));
      setKeyDrafts((prev) => ({ ...prev, [provider]: "" }));
      setKeyTests((prev) => ({ ...prev, [provider]: undefined }));
    } catch (error) {
      setProviderError(asFailure(error).detail);
    } finally {
      setSavingKey(null);
    }
  }

  async function testKey(provider: ProviderId) {
    setKeyTests((prev) => ({ ...prev, [provider]: { status: "testing" } }));
    try {
      await testProvider(provider);
      setKeyTests((prev) => ({ ...prev, [provider]: { status: "ok" } }));
    } catch (error) {
      setKeyTests((prev) => ({
        ...prev,
        [provider]: { status: "failed", failure: asFailure(error) },
      }));
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
          ? `Hold ${trigger} and speak; release and the words land where your cursor is. The audio goes to ${providerLabel(activeProvider)} with your key and nowhere else.`
          : `Hold the shortcut and speak; the words land where your cursor is. The audio goes to ${providerLabel(activeProvider)} with your key and nowhere else.`}
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

      <dl className="rows speech-rows">
        <div className="row">
          <dt className="row-label">Provider</dt>
          <dd className="row-value">
            <div className="segmented" role="group" aria-label="Active provider">
              {PROVIDERS.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  className="segmented-button"
                  aria-pressed={activeProvider === option.id}
                  disabled={switching || !snapshot}
                  onClick={() => void chooseProvider(option.id)}
                >
                  {option.label}
                </button>
              ))}
            </div>
            <p className="row-hint">
              Only the active provider receives your audio. Both keys stay configured.
            </p>
          </dd>
        </div>

        {PROVIDERS.map((option) => {
          const saved =
            option.id === "groq"
              ? snapshot?.config.providers.groq_key
              : snapshot?.config.providers.elevenlabs_key;
          const result = keyTests[option.id];
          return (
            <div className="row" key={option.id}>
              <dt className="row-label">{option.label} key</dt>
              <dd className="row-value">
                <div className="key-row">
                  <input
                    type="password"
                    className="text-input"
                    aria-label={`${option.label} API key`}
                    autoComplete="off"
                    spellCheck={false}
                    placeholder={
                      saved ? "Paste a new key to replace it" : `Paste your ${option.label} key`
                    }
                    value={keyDrafts[option.id] ?? ""}
                    disabled={!snapshot || savingKey === option.id}
                    onChange={(event) =>
                      setKeyDrafts((prev) => ({
                        ...prev,
                        [option.id]: event.target.value,
                      }))
                    }
                  />
                  <button
                    type="button"
                    className="button button-quiet"
                    disabled={!keyDrafts[option.id]?.trim() || savingKey === option.id}
                    onClick={() => void saveKey(option.id)}
                  >
                    {savingKey === option.id ? "Saving…" : "Save"}
                  </button>
                  <button
                    type="button"
                    className="button button-quiet"
                    disabled={!saved || result?.status === "testing"}
                    onClick={() => void testKey(option.id)}
                  >
                    {result?.status === "testing" ? "Testing…" : "Test"}
                  </button>
                </div>
                <p className="row-hint">
                  {saved ? "A key is saved. Paste a new one to replace it." : "No key saved yet."}
                </p>
                {result?.status === "ok" && (
                  <p className="key-test-ok">{option.label} accepted the key.</p>
                )}
                {result?.status === "failed" && result.failure && (
                  <p className="key-test-failed">
                    <span className="key-test-reason">{result.failure.reason}</span>{" "}
                    <span className="key-test-detail">{result.failure.detail}</span>
                  </p>
                )}
              </dd>
            </div>
          );
        })}
      </dl>

      {providerError && (
        <div className="notice" role="alert">
          <p className="notice-detail">{providerError}</p>
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
            Add a {providerLabel(activeProvider)} key above to dictate. Keys
            never leave this machine except to the provider.
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

      {phase === "failed" && (dictation?.failure || dictation?.message) && (
        <div className="notice" role="alert">
          <p className="notice-title">
            {dictation?.failure?.reason ?? "That dictation did not go through."}
          </p>
          <p className="notice-detail">
            {dictation?.failure?.detail ?? dictation?.message}
          </p>
          {dictation?.failure?.cause && (
            <p className="notice-detail mono">{dictation.failure.cause}</p>
          )}
          {dictation?.retryId && (
            <button
              className="button button-quiet"
              type="button"
              onClick={() => void retryLast()}
              disabled={retryingDictation}
            >
              {retryingDictation ? "Retrying…" : "Retry the recording"}
            </button>
          )}
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
  const shown = attempt.cleaned || attempt.transcript;
  const rawDiffers =
    Boolean(attempt.transcript) && attempt.transcript !== shown;

  return (
    <div className="result">
      <blockquote className="transcript">
        {shown || "No words came back. Try speaking a little closer."}
      </blockquote>
      {rawDiffers && (
        <p className="row-hint">
          Raw transcript: <span className="mono">{attempt.transcript}</span>
        </p>
      )}
      <p className="result-meta">
        {providerLabel(attempt.provider)}{" "}
        <span className="mono">{attempt.model}</span>
        {attempt.cleanupModel && (
          <>
            {" + "}
            <span className="mono">{attempt.cleanupModel}</span>
          </>
        )}
        , {recorded} of audio, back in {formatMilliseconds(timings.totalMs)}.
      </p>
      <p className="timings">
        Stopping {formatMilliseconds(timings.stopMs)}, encoding{" "}
        {formatMilliseconds(timings.encodeMs)}, upload{" "}
        {formatMilliseconds(timings.uploadMs)}, provider{" "}
        {formatMilliseconds(timings.providerMs)}
        {timings.cleanupMs > 0 &&
          `, cleanup ${formatMilliseconds(timings.cleanupMs)}`}
        .
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
