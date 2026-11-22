import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AnimatePresence, MotionConfig, motion } from "motion/react";
import {
  cancelDictation,
  DICTATION_EVENT,
  hidePillWindow,
  keyLabel,
  openMainWindow,
  readAccess,
  readDictationState,
  readPillVisibility,
  retryDictation,
  setPillInputRegion,
  toggleDictation,
  type DictationState,
  type PillVisibility,
} from "./lib/api";

const BAR_WEIGHTS = [0.42, 0.7, 0.92, 1.0, 0.92, 0.7, 0.42];
const IDLE_BARS = [6, 10, 13, 10, 6];
const EASE_OUT = [0.23, 1, 0.32, 1] as const;

/**
 * The pill: a persistent, non-focusing floating bar at the bottom centre.
 *
 * Keeps a compact obsidian capsule at rest, floats a non-interactive shortcut
 * tooltip above on hover without stretching the pill, clips all transitions
 * inside the capsule, and syncs its exact bounding box to GTK's input region
 * so everything outside the pill is click-through.
 */
export default function Pill() {
  const [dictation, setDictation] = useState<DictationState | null>(null);
  const [visibility, setVisibility] = useState<PillVisibility>("always");
  const [shortcut, setShortcut] = useState<string>("Right Ctrl");
  const [hovered, setHovered] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [bars, setBars] = useState<number[]>(() => BAR_WEIGHTS.map(() => 0.16));
  const since = useRef<number | null>(null);
  const pillRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let cancelled = false;

    readDictationState().then(
      (state) => {
        if (!cancelled) setDictation(state);
      },
      () => {},
    );

    readPillVisibility().then(
      (mode) => {
        if (!cancelled) setVisibility(mode);
      },
      () => {},
    );

    readAccess().then(
      (access) => {
        if (!cancelled && access.hotkey.trigger) {
          setShortcut(keyLabel(access.hotkey.trigger));
        }
      },
      () => {},
    );

    const unlistenState = listen<DictationState>(DICTATION_EVENT, (event) => {
      setDictation(event.payload);
      if (event.payload.phase !== "failed" && event.payload.phase !== "hint") {
        setDismissed(false);
      }
    });

    const unlistenVisibility = listen<PillVisibility>(
      "pill://visibility",
      (event) => {
        setVisibility(event.payload);
      },
    );

    const unlistenLevel = listen<number>("dictation://level", (event) => {
      const level = Math.max(0.08, Math.min(1, event.payload));
      setBars((prev) =>
        BAR_WEIGHTS.map((weight, index) => {
          const jitter = 0.85 + ((index * 37) % 30) / 100;
          const target = Math.min(1, level * weight * jitter);
          return prev[index] * 0.35 + target * 0.65;
        }),
      );
    });

    return () => {
      cancelled = true;
      void unlistenState.then((stop) => stop());
      void unlistenVisibility.then((stop) => stop());
      void unlistenLevel.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    if (dictation?.phase !== "listening") {
      since.current = null;
      setElapsed(0);
      setBars(BAR_WEIGHTS.map(() => 0.16));
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

  const rawPhase = dictation?.phase ?? "idle";
  const isError = (rawPhase === "failed" || rawPhase === "hint") && !dismissed;
  const phase =
    dismissed && (rawPhase === "failed" || rawPhase === "hint")
      ? "idle"
      : rawPhase;
  const failure = dictation?.failure ?? null;
  const retryId = dictation?.retryId ?? null;
  /** The short reason comes from Rust; the pill never invents one. */
  const shortReason =
    failure?.reason ?? dictation?.message ?? "Something went wrong.";

  const isIdle = phase === "idle";
  const isListening = phase === "listening";
  const isWorking = phase === "working";
  const isDone = phase === "done";
  const stageHidden =
    visibility === "disabled" || (visibility === "active_only" && isIdle);

  useEffect(() => {
    const el = pillRef.current;
    if (!el) return;

    const syncRegion = () => {
      if (stageHidden) {
        void setPillInputRegion(0, 0, 0, 0);
        return;
      }
      const rect = el.getBoundingClientRect();
      void setPillInputRegion(rect.left, rect.top, rect.width, rect.height);
    };

    syncRegion();
    const observer = new ResizeObserver(syncRegion);
    observer.observe(el);
    window.addEventListener("resize", syncRegion);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", syncRegion);
    };
  }, [stageHidden, phase, hovered]);

  function handlePillClick() {
    if (isIdle || isListening) {
      void toggleDictation();
    }
  }

  return (
    <MotionConfig reducedMotion="user">
      <motion.div
        className="pill-stage"
        initial={false}
        animate={
          stageHidden
            ? { opacity: 0, scale: 0.95 }
            : { opacity: 1, scale: 1 }
        }
        transition={{ duration: 0.14, ease: EASE_OUT }}
        onAnimationComplete={() => {
          if (visibility === "active_only" && isIdle) {
            void hidePillWindow();
          }
        }}
      >
        <AnimatePresence>
          {isIdle && hovered && !stageHidden && (
            <div className="pill-tooltip-anchor" aria-hidden="false">
              <motion.div
                key="tooltip"
                className="pill-tooltip"
                role="tooltip"
                initial={{ opacity: 0, y: 4, scale: 0.96 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: 3, scale: 0.97 }}
                transition={{ duration: 0.14, ease: EASE_OUT }}
              >
                <span>Click or hold</span>
                <kbd className="pill-kbd">{shortcut}</kbd>
              </motion.div>
            </div>
          )}
        </AnimatePresence>

        <motion.div
          ref={pillRef}
          className={[
            "pill",
            `pill-${phase}`,
            isIdle && hovered ? "pill-hover" : "",
            isIdle || isListening ? "pill-clickable" : "",
          ]
            .filter(Boolean)
            .join(" ")}
          role="status"
          aria-live="polite"
          onMouseEnter={() => setHovered(true)}
          onMouseLeave={() => setHovered(false)}
          onClick={handlePillClick}
          whileTap={isIdle || isListening ? { scale: 0.97 } : undefined}
          transition={{ duration: 0.12, ease: EASE_OUT }}
        >
          <AnimatePresence mode="wait" initial={false}>
            {isIdle && (
              <motion.div
                key="idle-rest"
                className="pill-row pill-idle-row"
                initial={{ opacity: 0, scale: 0.96 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.96 }}
                transition={{ duration: 0.13, ease: EASE_OUT }}
              >
                <span
                  className={
                    hovered
                      ? "pill-idle-dot pill-idle-dot-active"
                      : "pill-idle-dot"
                  }
                  aria-hidden="true"
                />
                <span className="pill-idle-bars" aria-hidden="true">
                  {IDLE_BARS.map((height, index) => (
                    <span
                      key={index}
                      className="pill-idle-glyph"
                      style={{ height: `${height}px` }}
                    />
                  ))}
                </span>
              </motion.div>
            )}

            {isListening && (
              <motion.div
                key="listening"
                className="pill-row"
                initial={{ opacity: 0, scale: 0.96 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.96 }}
                transition={{ duration: 0.13, ease: EASE_OUT }}
              >
                <span className="pill-wave" aria-label="Listening">
                  {bars.map((value, index) => (
                    <span
                      key={index}
                      className="pill-bar"
                      style={{ transform: `scaleY(${0.22 + value * 0.78})` }}
                    />
                  ))}
                </span>
                <span className="pill-time" aria-hidden="true">
                  {formatElapsed(elapsed)}
                </span>
              </motion.div>
            )}

            {isWorking && (
              <motion.div
                key="working"
                className="pill-row"
                initial={{ opacity: 0, scale: 0.96 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.96 }}
                transition={{ duration: 0.13, ease: EASE_OUT }}
              >
                <span className="pill-spinner" aria-hidden="true" />
                <span className="pill-label">Transcribing…</span>
                <button
                  className="pill-cancel"
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    void cancelDictation();
                  }}
                >
                  Cancel
                </button>
              </motion.div>
            )}

            {isDone && (
              <motion.div
                key="done"
                className="pill-row"
                initial={{ opacity: 0, scale: 0.96 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.96 }}
                transition={{ duration: 0.13, ease: EASE_OUT }}
              >
                <span className="pill-check" aria-hidden="true">
                  ✓
                </span>
                <span className="pill-label">Inserted</span>
              </motion.div>
            )}

            {isError && (
              <motion.div
                key="error"
                className="pill-error-card"
                initial={{ opacity: 0, scale: 0.96 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.96 }}
                transition={{ duration: 0.15, ease: EASE_OUT }}
                onClick={(event) => event.stopPropagation()}
              >
                <div className="pill-error-head">
                  <span className="pill-error-title">
                    <span className="pill-mark" aria-hidden="true">
                      ·
                    </span>
                    {rawPhase === "hint" ? "Heads up" : "Dictation failed"}
                  </span>
                  <button
                    className="pill-dismiss"
                    type="button"
                    aria-label="Dismiss"
                    onClick={() => setDismissed(true)}
                  >
                    ✕
                  </button>
                </div>
                <p className="pill-error-message">{shortReason}</p>
                <div className="pill-error-actions">
                  {retryId ? (
                    <button
                      className="pill-action"
                      type="button"
                      onClick={() => {
                        setDismissed(true);
                        void retryDictation();
                      }}
                    >
                      Retry
                    </button>
                  ) : (
                    failure === null && (
                      <button
                        className="pill-action"
                        type="button"
                        onClick={() => {
                          setDismissed(true);
                          void toggleDictation();
                        }}
                      >
                        Try again
                      </button>
                    )
                  )}
                  <button
                    className="pill-action pill-action-secondary"
                    type="button"
                    onClick={() => {
                      setDismissed(true);
                      void openMainWindow();
                    }}
                  >
                    Open settings
                  </button>
                </div>
              </motion.div>
            )}
          </AnimatePresence>
        </motion.div>
      </motion.div>
    </MotionConfig>
  );
}

function formatElapsed(milliseconds: number): string {
  return `${(milliseconds / 1000).toFixed(1)} s`;
}
