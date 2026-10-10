// The default progress window: a live speed graph, like Steam's.

import type { ReactNode } from "react";

import { niceMax } from "../components/Charts";
import { useI18n } from "../context";
import { formatSpeed } from "../format";
import { Icon, categoryIcon } from "../icons";
import { hostOf } from "../model";
import {
  DontShowAgain,
  type PwProps,
  SegmentBar,
  SpeedChart,
  WhenDoneChecks,
  WindowControls,
  phaseOf,
  useFigures,
  usePhaseText,
} from "./parts";

export function GraphView(p: PwProps) {
  const { t, language } = useI18n();
  const { d } = p;
  const phase = phaseOf(d);
  const f = useFigures(d);
  const phaseText = usePhaseText();
  const done = phase === "done";
  const busy = phase === "running" || phase === "processing" || phase === "checking" || phase === "connecting" || phase === "queued";

  let left: ReactNode;
  let right: ReactNode;
  if (done) {
    left = (
      <>
        <div className="pg-big done">
          <Icon name="tick" size={24} /> {t("pwDone")}
        </div>
        <div className="pg-sub">{f.took ? t("pwTook", { size: f.size, time: f.took }) : f.size}</div>
      </>
    );
    right = f.average && (
      <>
        <div className="pg-speed">{f.average}</div>
        <div className="pg-sub">{t("pwAverage")}</div>
      </>
    );
  } else {
    left = (
      <>
        <div className="pg-big">
          {f.pct == null ? f.done : f.pct}
          {f.pct != null && <small>%</small>}
        </div>
        <div className="pg-sub">{f.pct == null ? t("unknownSize") : f.of}</div>
      </>
    );
    right =
      phase === "running" ? (
        <>
          <div className="pg-speed">{f.speed}</div>
          {p.peak > 0 && <div className="pg-sub">{t("pwPeak", { speed: formatSpeed(p.peak, language) })}</div>}
        </>
      ) : (
        <div className={`pg-state ${phase}`}>{phaseText(phase)}</div>
      );
  }

  return (
    <div className={`pw-graph ${phase}`}>
      <header className="pg-head" data-tauri-drag-region>
        <span className={`pw-tile${done ? " good" : ""}`} data-tauri-drag-region>
          <Icon name={categoryIcon[d.category]} size={17} />
        </span>
        <div className="pg-title" data-tauri-drag-region>
          <div className="pw-name" title={d.fileName} data-tauri-drag-region>
            {d.fileName}
          </div>
          <div className="pw-host" data-tauri-drag-region>
            {hostOf(d.url)}
          </div>
        </div>
        <WindowControls onClose={p.act.close} />
      </header>

      <div className="pg-chart" data-tauri-drag-region>
        <SpeedChart values={p.speeds} max={niceMax(p.peak)} live={phase === "running"} />
        <div className="pg-overlay left">{left}</div>
        <div className="pg-overlay right">{right}</div>
      </div>

      <div className="pg-body">
        <SegmentBar d={d} />
        <div className="pg-caption">
          {done ? (
            <span className="pw-path" title={d.path}>
              {d.path}
            </span>
          ) : phase === "failed" ? (
            <span className="pw-error">{d.error ?? p.error}</span>
          ) : (
            <>
              <span>{phase === "running" && f.left ? <b>{t("pwLeftLong", { t: f.left })}</b> : phaseText(phase)}</span>
              <span>
                {phase === "running" && `${t(d.torrent ? "peersTitle" : "connectionsTitle", { n: d.connections })} · `}
                {d.resumable ? t("resumableYes") : t("resumableNo")}
              </span>
            </>
          )}
        </div>
      </div>

      <footer className="pw-foot">
        <div className="pw-checks">
          {done ? <DontShowAgain {...p} /> : <WhenDoneChecks {...p} />}
        </div>
        {done ? (
          <>
            <button type="button" className="button" onClick={p.act.folder}>
              <Icon name="folder" size={15} /> {t("openFolder")}
            </button>
            <button type="button" className="button primary" onClick={p.act.open}>
              <Icon name="open" size={15} /> {t("open")}
            </button>
          </>
        ) : (
          <>
            <button type="button" className="button" title={t("pwCancelHint")} onClick={p.act.cancel}>
              {t("pwCancel")}
            </button>
            {busy ? (
              <button type="button" className="button primary" onClick={p.act.pause}>
                <Icon name="pause" size={14} /> {t("pause")}
              </button>
            ) : (
              <button type="button" className="button primary" disabled={phase === "stopping"} onClick={p.act.resume}>
                <Icon name={phase === "failed" ? "retry" : "play"} size={14} /> {phase === "failed" ? t("actRetry") : t("resume")}
              </button>
            )}
          </>
        )}
      </footer>
    </div>
  );
}
