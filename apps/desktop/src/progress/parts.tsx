// Pieces shared by the progress window styles.

import { type Download, type WhenDone, api } from "../api";
import { useI18n } from "../context";
import { formatBytes, formatHuman, formatSpeed, percent } from "../format";
import { Icon } from "../icons";

/** What the progress window is showing. */
export interface PwProps {
  d: Download;
  /** Total speed, one sample every half second. */
  speeds: number[];
  peak: number;
  /** Smoothed speed of each unfinished segment, keyed by its first byte. */
  segmentSpeeds: Map<number, number>;
  whenDone: WhenDone | null;
  setWhenDone: (choice: WhenDone) => void;
  completeDialog: boolean;
  setCompleteDialog: (show: boolean) => void;
  details: boolean;
  toggleDetails: () => void;
  error: string | null;
  act: {
    pause: () => void;
    resume: () => void;
    /** Stops the download and closes the window. */
    cancel: () => void;
    open: () => void;
    folder: () => void;
    close: () => void;
  };
}

export type Phase = "connecting" | "checking" | "running" | "processing" | "stopping" | "queued" | "paused" | "failed" | "done";

export function phaseOf(d: Download): Phase {
  if (d.state === "completed") return "done";
  if (d.stopping) return "stopping";
  if (d.state === "downloading" && d.processing) return "processing";
  if (d.state === "downloading" && d.checking) return "checking";
  if (d.state === "downloading") return d.connections === 0 && d.speed === 0 ? "connecting" : "running";
  return d.state;
}

export function usePhaseText() {
  const { t } = useI18n();
  return (phase: Phase) =>
    ({
      connecting: t("connecting"),
      checking: t("checkingData"),
      running: t("state.downloading"),
      processing: t("processing"),
      stopping: t("stopping"),
      queued: t("state.queued"),
      paused: t("pwPaused"),
      failed: t("pwFailed"),
      done: t("pwDone"),
    })[phase];
}

/** Numbers every style shows. */
export function useFigures(d: Download) {
  const { t, language } = useI18n();
  const value = percent(d.downloaded, d.size);
  const seconds = d.elapsed / 1000;
  return {
    value,
    pct: value == null ? null : Math.floor(value),
    size: d.size == null ? t("unknownSize") : formatBytes(d.size, language),
    done: formatBytes(d.downloaded, language),
    of: d.size == null ? formatBytes(d.downloaded, language) : t("pwOf", { done: formatBytes(d.downloaded, language), size: formatBytes(d.size, language) }),
    speed: formatSpeed(d.speed, language),
    left: d.eta == null ? null : formatHuman(d.eta, language),
    took: seconds >= 1 ? formatHuman(seconds, language) : null,
    average: seconds >= 1 ? formatSpeed((d.size ?? d.downloaded) / seconds, language) : null,
  };
}

export function WindowControls({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  return (
    <div className="pw-controls">
      <button type="button" title={t("pwMinimize")} aria-label={t("pwMinimize")} onClick={() => api.minimizeWindow().catch(() => {})}>
        <Icon name="minus" size={14} />
      </button>
      <button type="button" title={t("close")} aria-label={t("close")} onClick={onClose}>
        <Icon name="x" size={14} />
      </button>
    </div>
  );
}

export function Check(props: { label: string; checked: boolean; onChange: (checked: boolean) => void }) {
  return (
    <label className="pw-check">
      <input type="checkbox" checked={props.checked} onChange={(e) => props.onChange(e.target.checked)} />
      <span>{props.label}</span>
    </label>
  );
}

/** "Show a dialog when done" and "Open the file when done". */
export function WhenDoneChecks({ whenDone, setWhenDone }: Pick<PwProps, "whenDone" | "setWhenDone">) {
  const { t } = useI18n();
  if (!whenDone) return null;
  return (
    <>
      <Check label={t("pwShowDialog")} checked={whenDone.showDialog} onChange={(showDialog) => setWhenDone({ ...whenDone, showDialog })} />
      <Check label={t("pwOpenFile")} checked={whenDone.openFile} onChange={(openFile) => setWhenDone({ ...whenDone, openFile })} />
    </>
  );
}

export function DontShowAgain({ completeDialog, setCompleteDialog }: Pick<PwProps, "completeDialog" | "setCompleteDialog">) {
  const { t } = useI18n();
  return <Check label={t("pwDontShowAgain")} checked={!completeDialog} onChange={(hide) => setCompleteDialog(!hide)} />;
}

/** The file as the server splits it: one block per segment, filled as far as it got. */
export function SegmentBar({ d, className = "" }: { d: Download; className?: string }) {
  const phase = phaseOf(d);
  const value = percent(d.downloaded, d.size);
  if (phase === "done" || !d.size || d.segments.length === 0) {
    const width = phase === "done" ? 100 : (value ?? 0);
    const unknown = phase !== "done" && value == null;
    return (
      <div className={`pw-segments plain ${phase} ${unknown ? "unknown" : ""} ${className}`}>
        <i style={{ left: 0, width: "100%" }}>
          <b style={{ width: unknown ? "100%" : `${width}%` }} />
        </i>
      </div>
    );
  }
  const size = d.size;
  return (
    <div className={`pw-segments ${phase} ${className}`}>
      {d.segments.map((s) => (
        <i key={s.start} style={{ left: `${(s.start / size) * 100}%`, width: `${((s.end - s.start) / size) * 100}%` }}>
          <b style={{ width: `${Math.min(100, (s.downloaded / Math.max(1, s.end - s.start)) * 100)}%` }} />
        </i>
      ))}
    </div>
  );
}

/** Speed over the last minute; the newest sample sits at the right edge. */
export function SpeedChart({ values, max, live, slots = 120 }: { values: number[]; max: number; live: boolean; slots?: number }) {
  const width = 480;
  const height = 120;
  const top = height * 0.42;
  const shown = values.slice(-slots);
  // Spread the first samples over the whole width instead of a sliver at the right.
  const step = width / (Math.max(24, shown.length) - 1);
  const flat = shown.every((v) => v === 0);
  const x = (i: number) => width - (shown.length - 1 - i) * step;
  const y = (v: number) => height - 6 - Math.min(1, v / Math.max(1, max)) * (height - 6 - top);
  const line = shown.map((v, i) => `${i ? "L" : "M"}${x(i).toFixed(1)} ${y(v).toFixed(1)}`).join(" ");
  const last = shown.length > 0 ? { x: x(shown.length - 1), y: y(shown[shown.length - 1]) } : null;
  return (
    <svg className="pw-chart" viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" aria-hidden="true">
      <defs>
        <linearGradient id="pw-chart-fill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="currentColor" stopOpacity="0.3" />
          <stop offset="1" stopColor="currentColor" stopOpacity="0" />
        </linearGradient>
      </defs>
      {[0.25, 0.5, 0.75].map((f) => (
        <line key={f} x1="0" x2={width} y1={height * f} y2={height * f} className="pw-grid" />
      ))}
      {shown.length > 1 && !flat && (
        <>
          <path d={`${line} L${width} ${height} L${x(0).toFixed(1)} ${height} Z`} fill="url(#pw-chart-fill)" />
          <path d={line} fill="none" stroke="currentColor" strokeWidth="2" vectorEffect="non-scaling-stroke" className="pw-glow" />
        </>
      )}
      {live && last && shown.length > 1 && !flat && <circle cx={last.x} cy={last.y} r="3.5" fill="currentColor" className="pw-glow pw-dot" />}
    </svg>
  );
}

/** A round progress gauge. */
export function Ring({ value, phase, size = 116 }: { value: number | null; phase: Phase; size?: number }) {
  const stroke = 10;
  const r = (size - stroke) / 2 - 2;
  const length = 2 * Math.PI * r;
  const shown = phase === "done" ? 100 : (value ?? 0);
  return (
    <svg className={`pw-ring ${phase}`} width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
      <defs>
        <linearGradient id="pw-ring-fill" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="var(--pw-g1)" />
          <stop offset="1" stopColor="var(--pw-g2)" />
        </linearGradient>
      </defs>
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" className="pw-ring-track" strokeWidth={stroke} />
      {value == null && phase !== "done" ? (
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke="url(#pw-ring-fill)"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={`${length * 0.22} ${length}`}
          className="pw-ring-spin"
        />
      ) : (
        <circle
          cx={size / 2}
          cy={size / 2}
          r={r}
          fill="none"
          stroke="url(#pw-ring-fill)"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray={`${(length * shown) / 100} ${length}`}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
          className="pw-ring-arc"
        />
      )}
    </svg>
  );
}

/** Per-connection speeds, two columns. */
export function ConnectionList({ d, segmentSpeeds }: Pick<PwProps, "d" | "segmentSpeeds">) {
  const { t, language } = useI18n();
  const running = d.segments.filter((s) => s.start + s.downloaded < s.end);
  const fastest = Math.max(1, ...running.map((s) => segmentSpeeds.get(s.start) ?? 0));
  return (
    <div className="pw-conns">
      {running.length === 0 && <span className="pw-dim">{t("connecting")}</span>}
      {running.map((s, i) => {
        const speed = segmentSpeeds.get(s.start) ?? 0;
        return (
          <div key={s.start} className="pw-conn">
            <span className="pw-dim">{t("segmentN", { n: i + 1 })}</span>
            <span className="pw-conn-bar">
              <b style={{ width: `${(speed / fastest) * 100}%` }} />
            </span>
            <span>{formatSpeed(speed, language)}</span>
          </div>
        );
      })}
    </div>
  );
}
