import {
  useCallback,
  useEffect,
  useState,
  type FormEvent,
} from "react";
import { listen } from "@tauri-apps/api/event";
import {
  asFailure,
  DICTATION_EVENT,
  keyLabel,
  providerLabel,
  PROVIDERS,
  readDictationState,
  testProvider,
  toggleDictation,
  TRIGGER_KEYS,
  updateProvider,
  updateProviderKey,
  updateTriggerKey,
  type Access,
  type ConfigSnapshot,
  type DictationState,
  type Failure,
  type ProviderId,
} from "./lib/api";

type Test = {
  status: "idle" | "testing" | "ok" | "failed";
  failure?: Failure;
};

/**
 * The first run: a key, a real request that proves it, then one real
 * dictation. Nothing here can finish before both proofs have landed.
 */
export default function Wizard({
  initial,
  access,
  onSnapshot,
  onDone,
}: {
  initial: ConfigSnapshot;
  access: Access | null;
  onSnapshot: (snapshot: ConfigSnapshot) => void;
  onDone: () => void;
}) {
  const [current, setCurrent] = useState(initial);
  const [step, setStep] = useState(1);
  const [chosenProvider, setChosenProvider] = useState<ProviderId>(
    initial.config.active_provider,
  );
  const [key, setKey] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [test, setTest] = useState<Test>({ status: "idle" });
  const [triggerSaving, setTriggerSaving] = useState(false);
  const [triggerError, setTriggerError] = useState<string | null>(null);
  const [dictation, setDictation] = useState<DictationState | null>(null);
  const [dictationError, setDictationError] = useState<string | null>(null);
  const [dictationDone, setDictationDone] = useState(false);
  const [busy, setBusy] = useState(false);

  const provider = current.config.active_provider;
  const trigger = access?.hotkey.trigger ?? keyLabel(current.config.trigger_key);
  const phase = dictation?.phase ?? "idle";
  const recording = phase === "listening";
  const working = phase === "working";
  const shown = dictation?.attempt
    ? dictation.attempt.cleaned || dictation.attempt.transcript
    : "";
  // Text is injected before a cleanup warning is attached. A blocked injection
  // is the only failed phase that carries a retry id, so this is the proof that
  // the whole loop works — even when cleanup had no Groq key to run on.
  const inserted =
    (phase === "done" || phase === "failed") &&
    Boolean(shown.trim()) &&
    !dictation?.retryId;

  function apply(snapshot: ConfigSnapshot) {
    setCurrent(snapshot);
    onSnapshot(snapshot);
  }

  async function saveKey(event: FormEvent) {
    event.preventDefault();
    const trimmed = key.trim();
    if (!trimmed) return;

    setSaving(true);
    setSaveError(null);
    try {
      let snapshot = current;
      if (snapshot.config.active_provider !== chosenProvider) {
        // The picker changes the choice here; in Settings it has its own call.
        snapshot = await updateProvider(chosenProvider);
        apply(snapshot);
      }
      apply(await updateProviderKey(chosenProvider, trimmed));
      setTest({ status: "idle" });
      setStep(3);
    } catch (error) {
      setSaveError(asFailure(error).detail);
    } finally {
      setSaving(false);
    }
  }

  const runTest = useCallback(async () => {
    setTest({ status: "testing" });
    try {
      await testProvider(provider);
      setTest({ status: "ok" });
    } catch (error) {
      setTest({ status: "failed", failure: asFailure(error) });
    }
  }, [provider]);

  useEffect(() => {
    if (step === 3 && test.status === "idle") void runTest();
  }, [step, test.status, runTest]);

  useEffect(() => {
    let cancelled = false;
    readDictationState().then(
      (state) => {
        if (!cancelled) setDictation(state);
      },
      (error: unknown) => {
        if (!cancelled) setDictationError(String(error));
      },
    );

    const unlisten = listen<DictationState>(DICTATION_EVENT, (event) => {
      setDictation(event.payload);
    });

    return () => {
      cancelled = true;
      void unlisten.then(
        (stop) => stop(),
        (error: unknown) =>
          console.error("could not stop the dictation listener", error),
      );
    };
  }, []);

  useEffect(() => {
    if (inserted) setDictationDone(true);
  }, [inserted]);

  async function changeTrigger(next: string) {
    setTriggerSaving(true);
    setTriggerError(null);
    try {
      apply(await updateTriggerKey(next));
    } catch (error) {
      setTriggerError(asFailure(error).detail);
    } finally {
      setTriggerSaving(false);
    }
  }

  async function toggle() {
    setBusy(true);
    setDictationError(null);
    try {
      await toggleDictation();
    } catch (error) {
      setDictationError(asFailure(error).detail);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="wizard" key={step}>
      <p className="wizard-progress">
        Step {step} of 4
      </p>

      {step === 1 && (
        <>
          <h1 className="wizard-title">Welcome to Sotto</h1>
          <p className="wizard-deck">
            Hold a key, speak, and the words land where you type. Four short
            steps: a key, a live test, a shortcut, and one real dictation.
          </p>
          <div className="wizard-actions">
            <button className="button" type="button" onClick={() => setStep(2)}>
              Set up Sotto
            </button>
          </div>
        </>
      )}

      {step === 2 && (
        <>
          <h1 className="wizard-title">Add your API key</h1>
          <p className="wizard-deck">
            Sotto transcribes with your own key and sends audio nowhere else.
            Pick a provider and paste its key; adding the other one later never
            erases this one.
          </p>
          <div className="segmented wizard-provider" role="group" aria-label="Provider">
            {PROVIDERS.map((option) => (
              <button
                key={option.id}
                type="button"
                className="segmented-button"
                aria-pressed={chosenProvider === option.id}
                onClick={() => setChosenProvider(option.id)}
              >
                {option.label}
              </button>
            ))}
          </div>
          <form className="wizard-form" onSubmit={(event) => void saveKey(event)}>
            <label className="wizard-label" htmlFor="wizard-key">
              {providerLabel(chosenProvider)} API key
            </label>
            <input
              id="wizard-key"
              type="password"
              className="text-input"
              autoComplete="off"
              spellCheck={false}
              placeholder={chosenProvider === "groq" ? "gsk-…" : "sk_…"}
              value={key}
              onChange={(event) => setKey(event.target.value)}
            />
            <p className="wizard-note">
              Find it in your {chosenProvider === "groq" ? "Groq console" : "ElevenLabs dashboard"}.
              It is stored in a file only your account can read.
            </p>
            {saveError && (
              <div className="notice wizard-notice" role="alert">
                <p className="notice-detail">{saveError}</p>
              </div>
            )}
            <div className="wizard-actions">
              <button className="button" type="submit" disabled={saving || !key.trim()}>
                {saving ? "Saving…" : "Test this key"}
              </button>
            </div>
          </form>
        </>
      )}

      {step === 3 && (
        <>
          <h1 className="wizard-title">Testing your key</h1>
          <p className="wizard-deck">
            Sotto sends a short silent clip to {providerLabel(provider)}. A real
            request proves the key now, instead of on your first dictation.
          </p>

          {test.status === "testing" && (
            <p className="wizard-status">
              <span className="pill-spinner" aria-hidden="true" />
              Waiting for {providerLabel(provider)}…
            </p>
          )}

          {test.status === "ok" && (
            <p className="wizard-pass">
              <span className="wizard-check" aria-hidden="true">✓</span>
              {providerLabel(provider)} accepted the key.
            </p>
          )}

          {test.status === "failed" && test.failure && (
            <div className="notice wizard-notice" role="alert">
              <p className="notice-title">{test.failure.reason}</p>
              <p className="notice-detail">{test.failure.detail}</p>
              <div className="wizard-inline-actions">
                <button className="button button-quiet" type="button" onClick={() => void runTest()}>
                  Try again
                </button>
                <button className="button button-quiet" type="button" onClick={() => setStep(2)}>
                  Change key
                </button>
              </div>
            </div>
          )}

          <div className="wizard-actions">
            <button
              className="button"
              type="button"
              disabled={test.status !== "ok"}
              onClick={() => setStep(4)}
            >
              Choose a shortcut
            </button>
          </div>
        </>
      )}

      {step === 4 && (
        <>
          <h1 className="wizard-title">Pick a shortcut and dictate</h1>
          <p className="wizard-deck">
            Choose a key to hold, then press it and say a short sentence. Release
            and the words should appear wherever your cursor is.
          </p>

          <div className="wizard-shortcut">
            <label className="wizard-label" htmlFor="wizard-trigger">Trigger key</label>
            <select
              id="wizard-trigger"
              className="text-input wizard-select"
              value={current.config.trigger_key}
              disabled={triggerSaving}
              onChange={(event) => void changeTrigger(event.target.value)}
            >
              {TRIGGER_KEYS.map((option) => (
                <option key={option.id} value={option.id}>{option.label}</option>
              ))}
            </select>
            <p className="wizard-note">
              Bound as <strong>{trigger}</strong>. You can change it later in General.
            </p>
          </div>

          {access?.hotkey.problem && (
            <div className="notice wizard-notice" role="alert">
              <p className="notice-title">The shortcut is not bound.</p>
              <p className="notice-detail">{access.hotkey.problem}</p>
            </div>
          )}
          {access?.injection.problem && (
            <div className="notice wizard-notice" role="alert">
              <p className="notice-title">Text cannot be typed yet.</p>
              <p className="notice-detail">{access.injection.problem}</p>
            </div>
          )}

          <div className="wizard-dictation">
            <button
              className={recording ? "button button-live" : "button"}
              type="button"
              disabled={working || busy}
              onClick={() => void toggle()}
            >
              {recording && <span className="recording-dot" aria-hidden="true" />}
              {recording ? "Stop and insert" : working ? "Transcribing…" : "Start test dictation"}
            </button>
            <p className="wizard-note" aria-live="polite">
              {recording
                ? "Listening. Speak a sentence, then release the shortcut."
                : working
                  ? "The recording is on its way."
                  : `Press ${trigger} or use the button above.`}
            </p>
          </div>

          {dictationError && (
            <div className="notice wizard-notice" role="alert">
              <p className="notice-detail">{dictationError}</p>
            </div>
          )}

          {phase === "failed" &&
            !inserted &&
            (dictation?.failure || dictation?.message) && (
            <div className="notice wizard-notice" role="alert">
              <p className="notice-title">
                {dictation?.failure?.reason ?? "That dictation did not go through."}
              </p>
              <p className="notice-detail">
                {dictation?.failure?.detail ?? dictation?.message}
              </p>
              <div className="wizard-inline-actions">
                <button className="button button-quiet" type="button" onClick={() => void toggle()}>
                  Try the dictation again
                </button>
              </div>
            </div>
          )}

          {dictationDone && shown && (
            <div className="wizard-proof">
              <blockquote className="transcript">{shown}</blockquote>
              <p className="wizard-pass">
                <span className="wizard-check" aria-hidden="true">✓</span>
                That dictation landed. The loop is proven.
              </p>
              {inserted && dictation?.failure && (
                <p className="wizard-note">
                  {dictation.failure.reason} — {dictation.failure.detail}
                </p>
              )}
            </div>
          )}
          {triggerError && <p className="wizard-error">{triggerError}</p>}

          <div className="wizard-actions">
            <button
              className="button"
              type="button"
              disabled={!dictationDone}
              onClick={onDone}
            >
              Start dictating
            </button>
          </div>
        </>
      )}
    </div>
  );
}
