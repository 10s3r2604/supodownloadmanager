// The familiar IDM layout, cleaned up: details, a connection list and a
// separate "download complete" dialog.

import { useState } from "react";

import { logoSmall } from "../brand";
import { useI18n } from "../context";
import { formatBytes, formatSpeed } from "../format";
import { Icon } from "../icons";
import {
  DontShowAgain,
  type PwProps,
  SegmentBar,
  WhenDoneChecks,
  WindowControls,
  phaseOf,
  useFigures,
  usePhaseText,
} from "./parts";

export function ClassicView(p: PwProps) {
  const { d } = p;
  return phaseOf(d) === "done" ? <ClassicDone {...p} /> : <ClassicProgress {...p} />;
}

function TitleBar({ title, onClose }: { title: string; onClose: () => void }) {
  return (
    <header className="pc-title" data-tauri-drag-region>
      <img src={logoSmall} width={16} height={16} alt="" data-tauri-drag-region />
      <span data-tauri-drag-region>{title}</span>
      <WindowControls onClose={onClose} />
    </header>
  );
}

function ClassicProgress(p: PwProps) {
  const { t, language } = useI18n();
  const { d } = p;
  const phase = phaseOf(d);
  const f = useFigures(d);
  const phaseText = usePhaseText();
  const [tab, setTab] = useState<"status" | "whenDone">("status");
  const busy = phase === "running" || phase === "processing" || phase === "connecting" || phase === "queued";
  const showConnections = p.details;
  const fill = f.value ?? 0;

  return (
    <div className={`pw-classic ${phase}`}>
      <TitleBar title={`${f.pct == null ? "" : `${f.pct}% `}${d.fileName}`} onClose={p.act.close} />
      <nav className="pc-tabs" data-tauri-drag-region>
        <button type="button" className={tab === "status" ? "on" : ""} onClick={() => setTab("status")}>
          {t("pwTabStatus")}
        </button>
        <button type="button" className={tab === "whenDone" ? "on" : ""} onClick={() => setTab("whenDone")}>
          {t("pwTabWhenDone")}
        </button>
      </nav>

      {tab === "status" ? (
        <div className="pc-body">
          <dl className="pc-kv">
            <dt>{t("pwAddress")}</dt>
            <dd title={d.url}>{d.url}</dd>
            <dt>{t("pwStatus")}</dt>
            <dd className={`pc-status ${phase}`}>
              {phase === "running" ? t("pwReceiving") : phase === "failed" ? (d.error ?? p.error ?? phaseText(phase)) : phaseText(phase)}
            </dd>
            <dt>{t("pwSize")}</dt>
            <dd>{f.size}</dd>
            <dt>{t("pwDownloaded")}</dt>
            <dd>
              {f.done}
              {f.pct != null && ` (${f.pct}%)`}
            </dd>
            <dt>{t("pwSpeed")}</dt>
            <dd>{phase === "running" ? f.speed : "—"}</dd>
            <dt>{t("pwLeft")}</dt>
            <dd>{phase === "running" && f.left ? f.left : "—"}</dd>
            <dt>{t("pwResumeSupport")}</dt>
            <dd>{d.resumable ? t("yes") : t("no")}</dd>
          </dl>
          <div className={`pc-bar ${phase}${f.value == null ? " unknown" : ""}`}>
            <span className="pc-bar-text">{f.pct == null ? f.done : `${f.pct}%`}</span>
            <div className="pc-bar-fill" style={{ clipPath: `inset(0 ${f.value == null ? 0 : 100 - fill}% 0 0)` }}>
              <span className="pc-bar-text">{f.pct == null ? f.done : `${f.pct}%`}</span>
            </div>
          </div>
          {showConnections && (
            <>
              <SegmentBar d={d} className="pc-map" />
              <ConnectionTable {...p} language={language} />
            </>
          )}
        </div>
      ) : (
        <div className="pc-body pc-when">
          <WhenDoneChecks {...p} />
          <p className="pw-dim">{t("pwWhenDoneHint")}</p>
        </div>
      )}

      <footer className="pw-foot">
        <button type="button" className="pw-link" onClick={p.toggleDetails}>
          <Icon name="chevron" size={14} className={showConnections ? "flip" : undefined} />
          {showConnections ? t("pwHideDetails") : t("pwShowDetails")}
        </button>
        <span className="pw-space" />
        <button type="button" className="button" title={t("pwCancelHint")} onClick={p.act.cancel}>
          {t("pwCancel")}
        </button>
        {busy ? (
          <button type="button" className="button primary" onClick={p.act.pause}>
            <Icon name="pause" size={14} /> {t("pause")}
          </button>
        ) : (
          <button type="button" className="button primary" disabled={phase === "stopping"} onClick={p.act.resume}>
            <Icon name={phase === "failed" ? "retry" : "play"} size={14} />
            {phase === "failed" ? t("actRetry") : t("resume")}
          </button>
        )}
      </footer>
    </div>
  );
}

/** One row per connection, like IDM's list under the progress bar. */
function ConnectionTable({ d, segmentSpeeds, language }: PwProps & { language: Parameters<typeof formatBytes>[1] }) {
  const { t } = useI18n();
  return (
    <div className="pc-table">
      <div className="pc-row head">
        <span>#</span>
        <span>{t("pwDownloaded")}</span>
        <span>{t("pwSpeed")}</span>
        <span>{t("pwStatus")}</span>
      </div>
      <div className="pc-rows">
        {d.segments.length === 0 && (
          <div className="pc-row">
            <span>1</span>
            <span>{formatBytes(d.downloaded, language)}</span>
            <span>{d.state === "downloading" ? formatSpeed(d.speed, language) : "—"}</span>
            <span>{d.state === "downloading" ? t("pwReceiving") : t("pwWaiting")}</span>
          </div>
        )}
        {d.segments.map((s, i) => {
          const finished = s.start + s.downloaded >= s.end;
          const speed = segmentSpeeds.get(s.start);
          return (
            <div key={s.start} className={`pc-row${finished ? " finished" : ""}`}>
              <span>{i + 1}</span>
              <span>{formatBytes(s.downloaded, language)}</span>
              <span>{!finished && speed != null && d.state === "downloading" ? formatSpeed(speed, language) : "—"}</span>
              <span>{finished ? t("pwSegmentDone") : d.state === "downloading" ? t("pwReceiving") : t("pwWaiting")}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function ClassicDone(p: PwProps) {
  const { t } = useI18n();
  const { d } = p;
  const f = useFigures(d);
  return (
    <div className="pw-classic done">
      <TitleBar title={t("pwDone")} onClose={p.act.close} />
      <div className="pc-done" data-tauri-drag-region>
        <span className="pw-tile big good" data-tauri-drag-region>
          <Icon name="tick" size={26} />
        </span>
        <div data-tauri-drag-region>
          <div className="pc-done-title">{t("pwDone")}</div>
          <div className="pw-host">{[f.size, f.took, f.average].filter(Boolean).join(" · ")}</div>
        </div>
      </div>
      <dl className="pc-kv">
        <dt>{t("pwAddress")}</dt>
        <dd title={d.url}>{d.url}</dd>
        <dt>{t("pwSavedTo")}</dt>
        <dd title={d.path}>{d.path}</dd>
      </dl>
      <div className="pc-dont">
        <DontShowAgain {...p} />
      </div>
      <footer className="pw-foot">
        <span className="pw-space" />
        <button type="button" className="button" onClick={p.act.close}>
          {t("close")}
        </button>
        <button type="button" className="button" onClick={p.act.folder}>
          <Icon name="folder" size={15} /> {t("openFolder")}
        </button>
        <button type="button" className="button primary" onClick={p.act.open}>
          <Icon name="open" size={15} /> {t("open")}
        </button>
      </footer>
    </div>
  );
}
