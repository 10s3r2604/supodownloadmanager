// The small window that follows one download until it is done, like IDM's.

import { useEffect, useRef, useState } from "react";

import { type Download, type Settings, api } from "../api";
import { useAppearance } from "../appearance";
import { I18nContext, errorText, useI18n } from "../context";
import { formatBytes, formatDuration, formatSpeed, percent } from "../format";
import type { TextKey } from "../i18n";
import { Icon, categoryIcon } from "../icons";
import { emptyHistory, hostOf, recordHistory } from "../model";
import { SegmentMap } from "../skins/Neon";
import "./progress.css";

const WIDTH = 500;
const HEIGHT = 268;
/** Extra room for the connections list: a title plus one row per two connections. */
const DETAILS_TITLE = 44;
const DETAILS_ROW = 21;
const MAX_DETAIL_ROWS = 8;
const POLL_MS = 500;

export function ProgressWindow({ id }: { id: number }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [download, setDownload] = useState<Download | null | undefined>(undefined);
  const [details, setDetails] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const history = useRef(emptyHistory());
  const { i18n } = useAppearance(settings);
  const { t } = i18n;

  useEffect(() => {
    api.getSettings().then(setSettings).catch(() => {});
    let alive = true;
    const poll = async () => {
      try {
        const next = await api.getDownload(id);
        if (!alive) return;
        if (next) history.current = recordHistory(history.current, [next]);
        setDownload(next);
      } catch (e) {
        if (alive) setError(errorText(e));
      }
    };
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [id]);

  // Window title like IDM: "42% ubuntu.iso".
  const value = download ? percent(download.downloaded, download.size) : null;
  const title = !download
    ? "Supo Download Manager"
    : download.state === "completed"
      ? `${t("pwDone")} · ${download.fileName}`
      : `${value == null ? "" : `${Math.floor(value)}% `}${download.fileName}`;
  useEffect(() => {
    api.setWindowTitle(title).catch(() => {});
  }, [title]);

  const showDetails = details && download?.state === "downloading";
  const rows = Math.min(MAX_DETAIL_ROWS, Math.ceil((settings?.connections ?? 8) / 2));
  useEffect(() => {
    api.resizeWindow(WIDTH, showDetails ? HEIGHT + DETAILS_TITLE + rows * DETAILS_ROW : HEIGHT).catch(() => {});
  }, [showDetails, rows]);

  const close = () => api.closeWindow().catch(() => window.close());
  const run = (action: Promise<unknown>) => action.catch((e) => setError(errorText(e)));

  return (
    <I18nContext.Provider value={i18n}>
      <div className="pw">
        {download === undefined ? null : download === null ? (
          <div className="pw-gone">
            <p>{t("pwGone")}</p>
            <button type="button" className="button" onClick={close}>
              {t("close")}
            </button>
          </div>
        ) : (
          <Body
            d={download}
            details={showDetails}
            speeds={[...(history.current.segments.get(download.id) ?? new Map()).values()]}
            error={error}
            onToggleDetails={() => setDetails((v) => !v)}
            onPause={() => run(api.pause([download.id]))}
            onResume={() => run(api.resume([download.id]))}
            onOpen={() => run(api.open(download.id)).then(close)}
            onFolder={() => run(api.showInFolder(download.id))}
            onClose={close}
          />
        )}
      </div>
    </I18nContext.Provider>
  );
}

interface BodyProps {
  d: Download;
  details: boolean;
  speeds: number[];
  error: string | null;
  onToggleDetails: () => void;
  onPause: () => void;
  onResume: () => void;
  onOpen: () => void;
  onFolder: () => void;
  onClose: () => void;
}

function Body({ d, details, speeds, error, ...on }: BodyProps) {
  const { t, language } = useI18n();
  const value = percent(d.downloaded, d.size);
  const running = d.state === "downloading";
  const done = d.state === "completed";

  const head = (
    <header className="pw-head">
      <span className={`pw-icon ${done ? "done" : ""}`}>
        <Icon name={done ? "check" : categoryIcon[d.category]} size={20} />
      </span>
      <div className="pw-title">
        <div className="pw-name" title={d.fileName}>
          {d.fileName}
        </div>
        <div className="pw-url" title={d.url}>
          {hostOf(d.url)}
        </div>
      </div>
    </header>
  );

  if (done) {
    return (
      <>
        {head}
        <div className="pw-done">
          <div className="pw-done-title">{t("pwDone")}</div>
          <div className="pw-path" title={d.path}>
            {d.path}
          </div>
          <div className="pw-done-size mono">{formatBytes(d.size, language)}</div>
        </div>
        <footer className="pw-foot">
          <span className="pw-space" />
          <button type="button" className="button" onClick={on.onClose}>
            {t("close")}
          </button>
          <button type="button" className="button" onClick={on.onFolder}>
            <Icon name="folder" size={15} /> {t("openFolder")}
          </button>
          <button type="button" className="button primary" autoFocus onClick={on.onOpen}>
            <Icon name="open" size={15} /> {t("open")}
          </button>
        </footer>
      </>
    );
  }

  const status = d.stopping
    ? t("stopping")
    : running && d.connections === 0 && d.speed === 0
      ? t("connecting")
      : d.state === "failed"
        ? t("pwFailed")
        : d.state === "paused"
          ? t("pwPaused")
          : t(`state.${d.state}` as TextKey);

  return (
    <>
      {head}
      <div className="pw-progress">
        <span className="pw-pct mono">{value == null ? "—" : `${Math.floor(value)}%`}</span>
        <span className={`pw-status ${d.state}`}>{status}</span>
      </div>
      <SegmentMap d={d} />
      <dl className="pw-stats">
        <div>
          <dt>{t("pwSize")}</dt>
          <dd className="mono">{d.size == null ? t("unknown") : formatBytes(d.size, language)}</dd>
        </div>
        <div>
          <dt>{t("pwDownloaded")}</dt>
          <dd className="mono">{formatBytes(d.downloaded, language)}</dd>
        </div>
        <div>
          <dt>{t("pwSpeed")}</dt>
          <dd className="mono">{running ? formatSpeed(d.speed, language) : "—"}</dd>
        </div>
        <div>
          <dt>{t("pwLeft")}</dt>
          <dd className="mono">{running && d.eta != null ? formatDuration(d.eta) : "—"}</dd>
        </div>
        <div>
          <dt>{t("resumable")}</dt>
          <dd>{d.resumable ? t("yes") : t("no")}</dd>
        </div>
      </dl>
      {(d.error || error) && <p className="pw-error">{d.error ?? error}</p>}
      {details && (
        <div className="pw-conns">
          <div className="pw-conns-title">{t("connectionsTitle", { n: d.connections })}</div>
          <div className="pw-conns-grid">
            {speeds.length === 0 && <span className="pw-dim">{t("connecting")}</span>}
            {speeds.map((speed, i) => (
              <div key={i} className="pw-conn mono">
                <span>{t("segmentN", { n: i + 1 })}</span>
                <span className="bar">
                  <b style={{ width: `${(speed / Math.max(1, ...speeds)) * 100}%` }} />
                </span>
                <span>{formatSpeed(speed, language)}</span>
              </div>
            ))}
          </div>
        </div>
      )}
      <footer className="pw-foot">
        {running && (
          <button type="button" className="pw-link" onClick={on.onToggleDetails}>
            <Icon name="chevron" size={14} className={details ? "flip" : undefined} />
            {details ? t("pwHideDetails") : t("pwShowDetails")}
          </button>
        )}
        <span className="pw-space" />
        <button type="button" className="button" onClick={on.onClose}>
          {t("close")}
        </button>
        {running || d.state === "queued" ? (
          <button type="button" className="button primary" onClick={on.onPause}>
            <Icon name="pause" size={14} /> {t("pause")}
          </button>
        ) : (
          <button type="button" className="button primary" onClick={on.onResume}>
            <Icon name={d.state === "failed" ? "retry" : "play"} size={14} />
            {d.state === "failed" ? t("actRetry") : t("resume")}
          </button>
        )}
      </footer>
    </>
  );
}
