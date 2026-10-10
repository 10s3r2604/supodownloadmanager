// A compact progress window with a big progress ring.

import { useI18n } from "../context";
import { Icon } from "../icons";
import { hostOf } from "../model";
import {
  ConnectionList,
  DontShowAgain,
  type PwProps,
  Ring,
  SegmentBar,
  WhenDoneChecks,
  WindowControls,
  phaseOf,
  useFigures,
  usePhaseText,
} from "./parts";

export function RingView(p: PwProps) {
  const { t } = useI18n();
  const { d } = p;
  const phase = phaseOf(d);
  const f = useFigures(d);
  const phaseText = usePhaseText();
  const done = phase === "done";
  const busy = phase === "running" || phase === "connecting" || phase === "queued";

  const centre = done ? (
    <Icon name="tick" size={36} />
  ) : phase === "failed" ? (
    <Icon name="alert" size={34} />
  ) : (
    <>
      <div className="pr-pct">
        {f.pct == null ? "—" : f.pct}
        {f.pct != null && <small>%</small>}
      </div>
      <div className="pr-speed">{phase === "running" ? f.speed : phaseText(phase)}</div>
    </>
  );

  const eyebrow = done
    ? t("pwDone")
    : phase === "running"
      ? `${t("state.downloading")} · ${t("connectionsTitle", { n: d.connections })}`
      : phaseText(phase);

  return (
    <div className={`pw-ring-view ${phase}`}>
      <WindowControls onClose={p.act.close} />
      <div className="pr-main" data-tauri-drag-region>
        <div className="pr-gauge" data-tauri-drag-region>
          <Ring value={f.value} phase={phase} />
          <div className="pr-centre">{centre}</div>
        </div>
        <div className="pr-info">
          <div className={`pr-eyebrow ${phase}`}>{eyebrow}</div>
          <div className="pw-name" title={d.fileName}>
            {d.fileName}
          </div>
          {done ? (
            <div className="pw-host">
              {[f.size, f.took, f.average && `${t("pwAverage")} ${f.average}`].filter(Boolean).join(" · ")}
            </div>
          ) : phase === "failed" ? (
            <div className="pw-error">{d.error ?? p.error}</div>
          ) : (
            <>
              <div className="pw-host">{hostOf(d.url)}</div>
              <div className="pr-stats">
                <span>
                  <b>{f.done}</b>
                  {d.size != null && ` / ${f.size}`}
                </span>
                {phase === "running" && f.left && <span>{t("pwLeftLong", { t: f.left })}</span>}
              </div>
            </>
          )}
          <div className="pr-actions">
            {done ? (
              <>
                <button type="button" className="button primary" onClick={p.act.open}>
                  <Icon name="open" size={15} /> {t("open")}
                </button>
                <button type="button" className="button" onClick={p.act.folder}>
                  <Icon name="folder" size={15} /> {t("openFolder")}
                </button>
              </>
            ) : (
              <>
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
                <button type="button" className="button" title={t("pwCancelHint")} onClick={p.act.cancel}>
                  {t("pwCancel")}
                </button>
                {phase === "running" && (
                  <button type="button" className="pw-link" onClick={p.toggleDetails}>
                    {t("pwDetails")}
                    <Icon name="chevron" size={14} className={p.details ? "flip" : undefined} />
                  </button>
                )}
              </>
            )}
          </div>
        </div>
      </div>

      {p.details && phase === "running" && (
        <div className="pr-details">
          <SegmentBar d={d} />
          <ConnectionList d={d} segmentSpeeds={p.segmentSpeeds} />
        </div>
      )}

      <footer className="pr-foot">
        {done ? <DontShowAgain {...p} /> : <WhenDoneChecks {...p} />}
      </footer>
    </div>
  );
}
