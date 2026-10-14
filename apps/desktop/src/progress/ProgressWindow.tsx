// The small window that follows one download until it is done.

import { type ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { type Download, type ProgressStyle, type Settings, type WhenDone, api } from "../api";
import { useAppearance } from "../appearance";
import { I18nContext, errorText } from "../context";
import { percent } from "../format";
import { type History, emptyHistory, recordHistory } from "../model";
import { ClassicView } from "./ClassicView";
import { GraphView } from "./GraphView";
import { type PwProps, WindowControls } from "./parts";
import { RingView } from "./RingView";
import "./progress.css";

const POLL_MS = 500;
/** One minute of speed samples. */
const SAMPLES = 120;
const WIDTH: Record<ProgressStyle, number> = { graph: 480, ring: 460, classic: 520 };

export function ProgressWindow({ id }: { id: number }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [download, setDownload] = useState<Download | null | undefined>(undefined);
  const [whenDone, setWhenDoneState] = useState<WhenDone | null>(null);
  const [details, setDetails] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [speeds, setSpeeds] = useState<number[]>([]);
  const history = useRef<History>(emptyHistory());
  const content = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const cancelEscapeUntil = useRef(0);
  const { i18n } = useAppearance(settings);
  const { t } = i18n;
  const style = settings?.progressStyle ?? "graph";

  useEffect(() => {
    let alive = true;
    let polling = false;
    api.getSettings().then(
      (next) => {
        if (alive) {
          setSettings(next);
          setSettingsError(null);
        }
      },
      (e) => { if (alive) setSettingsError(errorText(e)); },
    );
    api.getWhenDone(id).then((next) => { if (alive) setWhenDoneState(next); }).catch(() => {});
    const poll = async () => {
      if (polling) return;
      polling = true;
      try {
        const next = await api.getDownload(id);
        if (!alive) return;
        if (next) {
          history.current = recordHistory(history.current, [next]);
          if (next.state !== "completed") setSpeeds((list) => [...list, next.speed].slice(-SAMPLES));
        }
        setDownload(next);
        setPollError(null);
      } catch (e) {
        if (alive) setPollError(errorText(e));
      } finally {
        polling = false;
      }
    };
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [id, attempt]);

  // The taskbar shows "42% ubuntu.iso".
  const value = download ? percent(download.downloaded, download.size) : null;
  const title = !download
    ? "Supo Download Manager"
    : download.state === "completed"
      ? `${t("pwDone")} · ${download.fileName}`
      : `${value == null ? "" : `${Math.floor(value)}% `}${download.fileName}`;
  useEffect(() => {
    document.title = title;
    api.setWindowTitle(title).catch(() => {});
  }, [title]);

  // The window is as tall as what it shows.
  const width = WIDTH[style];
  useLayoutEffect(() => {
    const element = content.current;
    if (!element) return;
    let last = 0;
    const fit = () => {
      const height = Math.ceil(element.getBoundingClientRect().height);
      if (height > 0 && height !== last) {
        last = height;
        api.resizeWindow(width, height).catch(() => {});
      }
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => observer.disconnect();
  }, [width]);

  const close = useCallback(() => {
    api.closeWindow().catch(() => window.close());
  }, []);

  // Enter opens a finished file, Escape closes the window.
  const finished = download?.state === "completed";
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        // Windows can queue the cancellation key for WebView2 after the native
        // drag has ended. Consume that key and held-key repeats before closing.
        if (dragging.current || event.repeat || performance.now() < cancelEscapeUntil.current) {
          event.preventDefault();
          return;
        }
        close();
      } else if (event.key === "Enter" && finished && !dragging.current && !(event.target instanceof HTMLButtonElement)) {
        api.open(id).then(close, (e) => setError(errorText(e)));
      }
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (event.key === "Escape") cancelEscapeUntil.current = 0;
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [close, finished, id]);
  const run = (action: Promise<unknown>) => action.catch((e) => setError(errorText(e)));

  const setWhenDone = (choice: WhenDone) => {
    setWhenDoneState(choice);
    run(api.setWhenDone(id, choice));
  };
  const setCompleteDialog = (show: boolean) => {
    if (!settings) return;
    run(api.saveSettings({ ...settings, completeDialog: show }).then(setSettings));
  };

  let body: ReactNode = null;
  if (download === null) {
    body = (
      <div className="pw-gone" data-tauri-drag-region>
        <p>{t("pwGone")}</p>
        <button type="button" className="button" onClick={close}>
          {t("close")}
        </button>
      </div>
    );
  } else if (download && settings) {
    // The classic window shows its connection list unless it is hidden.
    const showDetails = details ?? style === "classic";
    const props: PwProps = {
      d: download,
      speeds,
      peak: Math.max(0, ...speeds),
      segmentSpeeds: history.current.segments.get(download.id) ?? new Map(),
      whenDone,
      setWhenDone,
      completeDialog: settings.completeDialog,
      setCompleteDialog,
      details: showDetails,
      toggleDetails: () => setDetails(!showDetails),
      error: error ?? pollError,
      act: {
        pause: () => run(api.pause([download.id])),
        resume: () => run(api.resume([download.id])),
        cancel: () => run(api.pause([download.id])).then(close),
        open: () => run(api.open(download.id)).then(close),
        drag: async () => {
          if (dragging.current) return;
          dragging.current = true;
          setError(null);
          try {
            if (!await api.dragDownload(download.id)) cancelEscapeUntil.current = performance.now() + 500;
          }
          catch { setError(t("pwDragFailed")); }
          finally { dragging.current = false; }
        },
        folder: () => run(api.showInFolder(download.id)),
        close,
      },
    };
    body =
      style === "ring" ? <RingView {...props} /> : style === "classic" ? <ClassicView {...props} /> : <GraphView {...props} />;
  } else {
    const startupError = settingsError ?? pollError;
    body = (
      <section className="pw-placeholder">
        <header className="pw-placeholder-head" data-tauri-drag-region>
          <strong data-tauri-drag-region>{t("pwTitle")}</strong>
          <WindowControls onClose={close} />
        </header>
        <div className="pw-placeholder-body" role={startupError ? "alert" : "status"}>
          {startupError ? (
            <>
              <strong>{t("pwLoadFailed")}</strong>
              <p>{startupError}</p>
              <button type="button" className="button" onClick={() => {
                setSettingsError(null);
                setPollError(null);
                setAttempt((n) => n + 1);
              }}>{t("pwRetry")}</button>
            </>
          ) : (
            <div className="pw-placeholder-status">
              <span className="spinner" aria-hidden="true" />
              <span>{t("pwLoading")}</span>
            </div>
          )}
        </div>
      </section>
    );
  }

  return (
    <I18nContext.Provider value={i18n}>
      <div className="pw">
        <div className="pw-content" ref={content}>
          {body}
          {finished && error && <p className="pw-action-error" role="alert">{error}</p>}
        </div>
      </div>
    </I18nContext.Provider>
  );
}
