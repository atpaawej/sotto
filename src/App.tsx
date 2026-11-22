import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  asFailure,
  checkForUpdates,
  copyHistory,
  deleteAllHistory,
  deleteHistory,
  DICTATION_EVENT,
  injectorLabel,
  installUpdate,
  keyLabel,
  normalizeShortcutCode,
  otherProvider,
  providerLabel,
  PROVIDERS,
  readAccess,
  readAppInfo,
  readConfig,
  readHistory,
  readInsights,
  readRecording,
  retryHistory,
  retryHistoryOther,
  retryKeyboardAccess,
  setStartAtLogin,
  startAtLoginEnabled,
  testProvider,
  updateCleanup,
  updateDictionary,
  updatePillVisibility,
  updateProvider,
  updateProviderKey,
  updateRetention,
  updateTriggerKey,
  type Access,
  type AppInfo,
  type Cleanup,
  type CleanupLevel,
  type ConfigSnapshot,
  type DictationState,
  type Failure,
  type HistoryEntry,
  type InsightsRange,
  type InsightsSnapshot,
  type PillVisibility,
  type ProviderId,
  type UpdateStatus,
} from "./lib/api";
import Wizard from "./Wizard";

type Loaded =
  | { status: "loading" }
  | { status: "ready"; snapshot: ConfigSnapshot; info: AppInfo }
  | { status: "failed"; message: string };

const SECTIONS = [
  { id: "insights", label: "Insights" },
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
  const [active, setActive] = useState<SectionId>("insights");
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
        <div className="brand-header">
          <img
            src="/logo.png"
            alt=""
            className="brand-logo"
            width={30}
            height={30}
          />
          <p className="wordmark">Sotto</p>
        </div>
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
    case "insights":
      return <InsightsSection />;

    case "general":
      return (
        <GeneralSection
          snapshot={snapshot}
          access={access}
          onSnapshot={onSnapshot}
        />
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
      return <AboutSection info={info} snapshot={snapshot} access={access} />;
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

function Switch({
  checked,
  disabled,
  label,
  onToggle,
}: {
  checked: boolean;
  disabled?: boolean;
  label: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className="switch"
      disabled={disabled}
      onClick={onToggle}
    >
      <span className="switch-thumb" aria-hidden="true" />
    </button>
  );
}

const INSIGHTS_RANGES: { id: InsightsRange; label: string }[] = [
  { id: "7d", label: "7 days" },
  { id: "30d", label: "30 days" },
  { id: "all", label: "All time" },
];

function formatDurationHuman(ms: number): string {
  const totalSeconds = Math.round(ms / 1000);
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) {
    return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  const remMinutes = minutes % 60;
  return remMinutes > 0 ? `${hours}h ${remMinutes}m` : `${hours}h`;
}

function buildHeatmapDays(
  dailyActivity: InsightsSnapshot["dailyActivity"],
  daysCount: number,
): { date: string; count: number; words: number; level: 0 | 1 | 2 | 3 }[] {
  const byDate = new Map(dailyActivity.map((item) => [item.date, item]));
  const maxWords = Math.max(1, ...dailyActivity.map((item) => item.words));
  const cells: { date: string; count: number; words: number; level: 0 | 1 | 2 | 3 }[] = [];
  const today = new Date();
  for (let offset = daysCount - 1; offset >= 0; offset -= 1) {
    const d = new Date(today.getFullYear(), today.getMonth(), today.getDate() - offset);
    const yyyy = d.getFullYear();
    const mm = String(d.getMonth() + 1).padStart(2, "0");
    const dd = String(d.getDate()).padStart(2, "0");
    const date = `${yyyy}-${mm}-${dd}`;
    const entry = byDate.get(date);
    const words = entry?.words ?? 0;
    const count = entry?.count ?? 0;
    let level: 0 | 1 | 2 | 3 = 0;
    if (words > 0) {
      const ratio = words / maxWords;
      level = ratio > 0.66 ? 3 : ratio > 0.33 ? 2 : 1;
    }
    cells.push({ date, count, words, level });
  }
  return cells;
}

function InsightsSection() {
  const [range, setRange] = useState<InsightsRange>("30d");
  const [data, setData] = useState<InsightsSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    readInsights(range).then(
      (snapshot) => {
        if (!cancelled) setData(snapshot);
      },
      (err: unknown) => {
        if (!cancelled) setError(asFailure(err).detail);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [range]);

  const heatmapDays = data ? buildHeatmapDays(data.dailyActivity, 84) : [];
  const successRate =
    data && data.totalDictations > 0
      ? Math.round((data.insertedCount / data.totalDictations) * 100)
      : 0;

  return (
    <section className="section">
      <div className="insights-header">
        <div>
          <h1 className="section-title">Insights</h1>
          <p className="section-deck">
            Your local dictation pace, streaks, and time saved.
          </p>
        </div>
        <div className="segmented" role="group" aria-label="Insights range">
          {INSIGHTS_RANGES.map((item) => (
            <button
              key={item.id}
              type="button"
              className="segmented-button"
              aria-pressed={range === item.id}
              onClick={() => setRange(item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>
      </div>

      {error && (
        <div className="notice" role="alert">
          <p className="notice-title">Could not load insights.</p>
          <p className="notice-detail">{error}</p>
        </div>
      )}

      {data && data.totalDictations === 0 ? (
        <p className="empty quiet">
          No dictations in this range yet. Hold your trigger shortcut to speak —
          your pace, streak, and time saved will appear here.
        </p>
      ) : data ? (
        <div className="insights-body">
          <div className="kpi-grid">
            <div className="kpi">
              <p className="kpi-label">Words dictated</p>
              <p className="kpi-value">{data.totalWords.toLocaleString()}</p>
              <p className="kpi-sub">
                {data.totalDictations}{" "}
                {data.totalDictations === 1 ? "dictation" : "dictations"}
              </p>
            </div>
            <div className="kpi">
              <p className="kpi-label">Average pace</p>
              <p className="kpi-value">{data.avgWpm} WPM</p>
              <p className="kpi-sub">
                {formatDurationHuman(data.totalSpeakingMs)} speaking
              </p>
            </div>
            <div className="kpi">
              <p className="kpi-label">Time saved</p>
              <p className="kpi-value">
                {formatDurationHuman(data.estimatedTimeSavedMs)}
              </p>
              <p className="kpi-sub">vs. 40 WPM typing</p>
            </div>
            <div className="kpi">
              <p className="kpi-label">Daily streak</p>
              <p className="kpi-value">
                {data.currentStreakDays}{" "}
                {data.currentStreakDays === 1 ? "day" : "days"}
              </p>
              <p className="kpi-sub">Best: {data.longestStreakDays} days</p>
            </div>
          </div>

          <div className="insights-block">
            <div className="insights-block-head">
              <h2 className="insights-heading">Activity</h2>
              <span className="insights-meta">Last 12 weeks</span>
            </div>
            <div className="heatmap" role="img" aria-label="Daily dictation activity">
              {heatmapDays.map((cell) => (
                <span
                  key={cell.date}
                  className={`heatmap-cell heatmap-level-${cell.level}`}
                  title={`${cell.date}: ${cell.words} words (${cell.count} ${cell.count === 1 ? "dictation" : "dictations"})`}
                />
              ))}
            </div>
          </div>

          <div className="insights-split">
            <div className="insights-block">
              <h2 className="insights-heading">Reliability &amp; Cleanup</h2>
              <dl className="insights-stats">
                <div className="insights-stat-row">
                  <dt>Inserted</dt>
                  <dd className="tabular">
                    {successRate}% ({data.insertedCount}/{data.totalDictations})
                  </dd>
                </div>
                <div className="insights-stat-row">
                  <dt>Words trimmed by Cleanup</dt>
                  <dd className="tabular">{data.wordsTrimmed.toLocaleString()}</dd>
                </div>
                {(data.failedCount > 0 || data.blockedCount > 0) && (
                  <div className="insights-stat-row">
                    <dt>Failed / blocked</dt>
                    <dd className="tabular">
                      {data.failedCount} failed · {data.blockedCount} blocked
                    </dd>
                  </div>
                )}
              </dl>
            </div>

            <div className="insights-block">
              <h2 className="insights-heading">Providers &amp; Latency</h2>
              {data.byProvider.length > 0 ? (
                <dl className="insights-stats">
                  {data.byProvider.map((item) => (
                    <div className="insights-stat-row" key={item.provider}>
                      <dt>
                        {providerLabel(item.provider)}{" "}
                        <span className="insights-meta">({item.count})</span>
                      </dt>
                      <dd className="tabular">
                        {item.avgTotalMs !== null
                          ? `${formatMilliseconds(item.avgTotalMs)} avg`
                          : "No timings yet"}
                      </dd>
                    </div>
                  ))}
                </dl>
              ) : (
                <p className="quiet">No provider activity yet.</p>
              )}
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}

function GeneralSection({
  snapshot,
  access,
  onSnapshot,
}: {
  snapshot?: ConfigSnapshot;
  access: Access | null;
  onSnapshot: (snapshot: ConfigSnapshot) => void;
}) {
  const [atLogin, setAtLogin] = useState(false);
  const [pillSaving, setPillSaving] = useState(false);
  const [recordingShortcut, setRecordingShortcut] = useState(false);
  const [pendingKeys, setPendingKeys] = useState<string[]>([]);
  const [shortcutSaving, setShortcutSaving] = useState(false);
  const [shortcutError, setShortcutError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    startAtLoginEnabled().then((enabled) => {
      if (!cancelled) setAtLogin(enabled);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const commitShortcut = useCallback(
    async (keys: string[]) => {
      const spec = keys.join("+");
      setRecordingShortcut(false);
      setPendingKeys([]);
      if (!spec) return;
      setShortcutSaving(true);
      setShortcutError(null);
      try {
        const updated = await updateTriggerKey(spec);
        onSnapshot(updated);
      } catch (err) {
        setShortcutError(String(err));
      } finally {
        setShortcutSaving(false);
      }
    },
    [onSnapshot],
  );

  useEffect(() => {
    if (!recordingShortcut) return;
    const held: string[] = [];

    function onKeyDown(event: KeyboardEvent) {
      event.preventDefault();
      event.stopPropagation();
      if (event.code === "Escape") {
        setRecordingShortcut(false);
        setPendingKeys([]);
        return;
      }
      const normalized = normalizeShortcutCode(event.code);
      if (!normalized) {
        setShortcutError(`Unsupported key "${event.key}". Try Ctrl, Alt, Shift, Super, Space, F1–F12, or A–Z.`);
        return;
      }
      setShortcutError(null);
      if (!held.includes(normalized) && held.length < 3) {
        held.push(normalized);
        setPendingKeys([...held]);
      }
      if (held.length === 3) {
        void commitShortcut(held);
      }
    }

    function onKeyUp(event: KeyboardEvent) {
      event.preventDefault();
      event.stopPropagation();
      if (held.length > 0) {
        void commitShortcut(held);
      }
    }

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [recordingShortcut, commitShortcut]);

  async function toggleAtLogin() {
    const next = !atLogin;
    setAtLogin(next);
    try {
      await setStartAtLogin(next);
    } catch {
      // The desktop state is the source of truth, so the switch just goes back.
      setAtLogin(!next);
    }
  }

  async function changePillVisibility(visibility: PillVisibility) {
    if (pillSaving) return;
    setPillSaving(true);
    try {
      const updated = await updatePillVisibility(visibility);
      onSnapshot(updated);
    } finally {
      setPillSaving(false);
    }
  }

  const pillVisibility = snapshot?.config.pill_visibility ?? "always";
  const currentTrigger = snapshot?.config.trigger_key ?? "ControlRight";

  return (
    <section className="section">
      <h1 className="section-title">General</h1>
      <p className="section-deck">
        Shortcut, startup, and floating pill behavior.
      </p>
      {snapshot ? (
        <dl className="rows">
          <div className="row">
            <dt className="row-label">Trigger shortcut</dt>
            <dd className="row-value">
              <div className="shortcut-control">
                <kbd className="shortcut-kbd">
                  {recordingShortcut
                    ? pendingKeys.length > 0
                      ? pendingKeys.map(keyLabel).join(" + ")
                      : "Press 1–3 keys…"
                    : keyLabel(currentTrigger)}
                </kbd>
                <button
                  type="button"
                  className="button button-quiet"
                  disabled={shortcutSaving}
                  onClick={() => {
                    setShortcutError(null);
                    setPendingKeys([]);
                    setRecordingShortcut((prev) => !prev);
                  }}
                >
                  {recordingShortcut ? "Cancel" : "Change"}
                </button>
                {!recordingShortcut && currentTrigger !== "ControlRight" && (
                  <button
                    type="button"
                    className="button button-quiet"
                    disabled={shortcutSaving}
                    onClick={() => void commitShortcut(["ControlRight"])}
                  >
                    Reset
                  </button>
                )}
              </div>
              <p className="row-hint">
                {recordingShortcut
                  ? "Hold 1–3 keys together and release to save, or press Esc to cancel."
                  : access?.hotkey.bound
                    ? "Hold to dictate anywhere. Double-tap to lock it on."
                    : "Not bound yet — Speech shows what went wrong."}
              </p>
              {shortcutError && (
                <p className="key-test-failed">{shortcutError}</p>
              )}
            </dd>
          </div>
          <div className="row">
            <dt className="row-label">Start at login</dt>
            <dd className="row-value">
              <Switch
                checked={atLogin}
                label="Start at login"
                onToggle={() => void toggleAtLogin()}
              />
              <p className="row-hint">Open Sotto automatically when you log in.</p>
            </dd>
          </div>
          <Row
            label="Floating pill"
            value={
              <div
                className="segmented"
                role="group"
                aria-label="Floating pill visibility"
              >
                {(
                  [
                    ["always", "Always"],
                    ["active_only", "While dictating"],
                    ["disabled", "Hidden"],
                  ] as const
                ).map(([value, label]) => (
                  <button
                    key={value}
                    type="button"
                    className="segmented-button"
                    aria-pressed={pillVisibility === value}
                    disabled={pillSaving}
                    onClick={() => void changePillVisibility(value)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            }
            hint={
              pillVisibility === "always"
                ? "Stays at the bottom centre as a compact bar."
                : pillVisibility === "active_only"
                  ? "Appears while listening, working, or showing a result."
                  : "Hidden at all times. Failures still appear in History."
            }
          />
        </dl>
      ) : (
        <p className="quiet">Settings are not available.</p>
      )}
    </section>
  );
}

type UpdatesState =
  | { status: "checking" }
  | { status: "up-to-date" }
  | { status: "available"; latest: string }
  | { status: "error" }
  | { status: "installed" };

function updatesState(status: UpdateStatus): UpdatesState {
  if (!status.available) return { status: "up-to-date" };
  return { status: "available", latest: status.latest ?? "" };
}

function AboutSection({
  info,
  snapshot,
  access,
}: {
  info?: AppInfo;
  snapshot?: ConfigSnapshot;
  access: Access | null;
}) {
  const [state, setState] = useState<UpdatesState>({ status: "checking" });
  const [installing, setInstalling] = useState(false);
  const [copiedPath, setCopiedPath] = useState(false);

  const runCheck = useCallback(() => {
    setState({ status: "checking" });
    checkForUpdates().then(
      (status) => setState(updatesState(status)),
      () => setState({ status: "error" }),
    );
  }, []);

  useEffect(() => {
    runCheck();
  }, [runCheck]);

  async function install() {
    setInstalling(true);
    try {
      await installUpdate();
      setState({ status: "installed" });
    } catch {
      setState({ status: "error" });
    } finally {
      setInstalling(false);
    }
  }

  async function copyConfigPath() {
    if (!snapshot) return;
    await navigator.clipboard.writeText(snapshot.path);
    setCopiedPath(true);
    window.setTimeout(() => setCopiedPath(false), 1600);
  }

  const text: Record<UpdatesState["status"], string> = {
    checking: "Checking…",
    "up-to-date": "Up to date",
    available: "Update available",
    error: "Could not check for updates.",
    installed: "Update installed",
  };

  const hint =
    state.status === "available"
      ? `Version ${state.latest} is out.`
      : state.status === "installed"
        ? "Restart Sotto to use it."
        : undefined;

  const injector = access?.injection.active || null;

  return (
    <section className="section">
      <h1 className="section-title">About</h1>
      <p className="section-deck">
        Sotto is open source, and the only computer it talks to is the speech
        provider you chose.
      </p>
      {info ? (
        <dl className="rows">
          <Row label="Version" value={<span>{info.version}</span>} />
          <Row label="License" value={info.license} />
          <Row label="Source" value={<span className="mono">{info.repository}</span>} />
          <div className="row">
            <dt className="row-label">Updates</dt>
            <dd className="row-value">
              <div className="shortcut-control">
                <span>{text[state.status]}</span>
                {state.status !== "checking" &&
                  state.status !== "installed" &&
                  !installing && (
                    <button
                      type="button"
                      className="button button-quiet"
                      onClick={runCheck}
                    >
                      Check for updates
                    </button>
                  )}
                {state.status === "available" && (
                  <button
                    type="button"
                    className="button"
                    onClick={() => void install()}
                    disabled={installing}
                  >
                    {installing ? "Installing…" : "Install update"}
                  </button>
                )}
              </div>
              {hint && <p className="row-hint">{hint}</p>}
            </dd>
          </div>
          {injector && (
            <Row
              label="Injector"
              value={injectorLabel(injector)}
              hint={
                access?.injection.reason || access?.injection.problem || undefined
              }
            />
          )}
          {snapshot && (
            <div className="row">
              <dt className="row-label">Settings file</dt>
              <dd className="row-value">
                <div className="shortcut-control">
                  <span className="mono">{snapshot.path}</span>
                  <button
                    type="button"
                    className="button button-quiet"
                    onClick={() => void copyConfigPath()}
                  >
                    {copiedPath ? "Copied" : "Copy"}
                  </button>
                </div>
                <p className="row-hint">
                  {snapshot.mode === "600"
                    ? "Mode 600. Only your user account can read it."
                    : `Mode ${snapshot.mode}.`}
                </p>
              </dd>
            </div>
          )}
        </dl>
      ) : null}
    </section>
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
        <Switch
          checked={on}
          disabled={disabled}
          label={label}
          onToggle={onToggle}
        />
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

  // A retry with the other provider starts on the session thread: its new row
  // and its result land after the click handler has returned.
  useEffect(() => {
    const unlisten = listen<DictationState>(DICTATION_EVENT, (event) => {
      if (event.payload.phase === "done" || event.payload.phase === "failed") {
        setReload((count) => count + 1);
      }
    });
    return () => {
      void unlisten.then(
        (stop) => stop(),
        (error: unknown) =>
          console.error("could not stop the history listener", error),
      );
    };
  }, []);

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
  const [retrying, setRetrying] = useState(false);
  const [keyDrafts, setKeyDrafts] = useState<Partial<Record<ProviderId, string>>>({});
  const [keyTests, setKeyTests] = useState<Partial<Record<ProviderId, KeyTest>>>({});
  const [savingKey, setSavingKey] = useState<ProviderId | null>(null);
  const [switching, setSwitching] = useState(false);
  const [providerError, setProviderError] = useState<string | null>(null);

  const activeProvider = snapshot?.config.active_provider ?? "groq";

  async function retryAccess() {
    setRetrying(true);
    try {
      await retryKeyboardAccess();
      window.setTimeout(refreshAccess, 800);
    } catch (error) {
      setProviderError(asFailure(error).detail);
    } finally {
      setRetrying(false);
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

  const trigger = access?.hotkey.trigger ? keyLabel(access.hotkey.trigger) : null;

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
    </section>
  );
}

function formatElapsed(milliseconds: number): string {
  return `${(milliseconds / 1000).toFixed(1)} s`;
}

function formatMilliseconds(milliseconds: number): string {
  return milliseconds < 1000
    ? `${Math.round(milliseconds)} ms`
    : `${(milliseconds / 1000).toFixed(2)} s`;
}
