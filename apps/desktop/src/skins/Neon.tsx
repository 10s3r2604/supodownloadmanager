// Neon: dark and detailed. Live speed chart, segment bars and connections.

import { useState } from "react";

import type { Download } from "../api";
import { logoSmall } from "../brand";
import { AreaChart, SegmentBlocks, Sparkline, niceMax } from "../components/Charts";
import { useI18n } from "../context";
import { formatBytes, formatDuration, formatHuman, formatSpeed, percent } from "../format";
import type { TextKey } from "../i18n";
import { Icon, type IconName, categoryIcon } from "../icons";
import { CATEGORIES, type Filter, blocks, hostOf, matches } from "../model";
import type { ViewProps } from "../views";
import "./neon.css";

const STATE_TABS: Filter[] = ["all", "downloading", "queued", "paused", "completed", "failed"];
const HISTORY_SLOTS = 100;

export function NeonView(p: ViewProps) {
  const { t, language } = useI18n();
  const [categoriesOpen, setCategoriesOpen] = useState(false);
  const count = (f: Filter) => p.downloads.filter((d) => matches(f, d)).length;
  const category = p.filter.startsWith("cat:") ? (p.filter.slice(4) as Download["category"]) : null;
  const selection = p.downloads.filter((d) => p.selected.includes(d.id));
  const detail = selection.length === 1 ? selection[0] : null;
  const max = niceMax(Math.max(0, ...p.history.total));
  const bridgeOk = p.info ? !p.info.bridgeError : true;

  return (
    <div className="neon">
      <aside className="neon-rail">
        <img src={logoSmall} className="neon-logo" alt="SDM" />
        <RailButton
          icon="list"
          label={t("downloadsTitle")}
          active={!category}
          onClick={() => {
            p.setFilter("all");
            setCategoriesOpen(false);
          }}
        />
        <RailButton
          icon="layers"
          label={t("categories")}
          active={Boolean(category) || categoriesOpen}
          onClick={() => setCategoriesOpen((open) => !open)}
        />
        <span className="neon-rail-space" />
        <RailButton
          icon="globe"
          label={p.info?.bridgeError ? t("bridgeError", { error: p.info.bridgeError }) : t("browsers")}
          dot={bridgeOk ? "ok" : "bad"}
          onClick={p.onBrowsers}
        />
        <RailButton icon="settings" label={t("settings")} onClick={p.onSettings} />

        {categoriesOpen && (
          <div className="neon-flyout" role="menu">
            <button
              type="button"
              className={!category ? "on" : ""}
              onClick={() => {
                p.setFilter("all");
                setCategoriesOpen(false);
              }}
            >
              <Icon name="list" size={16} /> {t("allCategories")}
              <span>{p.downloads.length}</span>
            </button>
            {CATEGORIES.map((c) => (
              <button
                type="button"
                key={c}
                className={category === c ? "on" : ""}
                onClick={() => {
                  p.setFilter(`cat:${c}`);
                  setCategoriesOpen(false);
                }}
              >
                <Icon name={categoryIcon[c]} size={16} /> {t(`cat.${c}` as TextKey)}
                <span>{count(`cat:${c}`) || ""}</span>
              </button>
            ))}
          </div>
        )}
      </aside>

      <div className="neon-main">
        <header className="neon-head">
          <h1>{t("downloadsTitle")}</h1>
          <nav className="neon-tabs">
            {STATE_TABS.map((f) => (
              <button type="button" key={f} className={p.filter === f ? "on" : ""} onClick={() => p.setFilter(f)}>
                {t(`tab.${f}` as TextKey)} <span>{count(f)}</span>
              </button>
            ))}
            {category && (
              <span className="neon-chip">
                <Icon name={categoryIcon[category]} size={13} />
                {t(`cat.${category}` as TextKey)}
                <button type="button" aria-label={t("dismiss")} onClick={() => p.setFilter("all")}>
                  <Icon name="x" size={12} />
                </button>
              </span>
            )}
          </nav>
          <label className="neon-search">
            <Icon name="search" size={14} />
            <input value={p.query} placeholder={t("filterPlaceholder")} onChange={(e) => p.setQuery(e.target.value)} />
          </label>
          <button type="button" className="neon-icon-btn" title={t("resumeAll")} onClick={p.onResumeAll}>
            <Icon name="playAll" size={16} />
          </button>
          <button type="button" className="neon-icon-btn" title={t("pauseAll")} onClick={p.onPauseAll}>
            <Icon name="pauseAll" size={16} />
          </button>
          <button type="button" className="neon-add" onClick={p.onAdd}>
            <Icon name="plus" size={15} /> {t("add")}
          </button>
        </header>

        <section className="neon-chart">
          <div className="neon-kpi">
            <div className="label">{t("totalSpeed")}</div>
            <div className="big mono">
              {(p.totals.speed / 1024 / 1024).toFixed(2)}
              <small>MB/s</small>
            </div>
            <dl>
              <div>
                <dd className="mono">{p.totals.connections}</dd>
                <dt>{t("connectionsKpi")}</dt>
              </div>
              <div>
                <dd className="mono">{formatBytes(p.totals.downloadedToday, language)}</dd>
                <dt>{t("downloadedToday")}</dt>
              </div>
              <div>
                <dd className="mono">{p.totals.remaining == null ? "—" : formatDuration(p.totals.remaining)}</dd>
                <dt>{t("timeLeft")}</dt>
              </div>
              <div>
                <dd className="mono">{p.totals.speedLimit ? formatSpeed(p.totals.speedLimit, language) : "∞"}</dd>
                <dt>{t("limit")}</dt>
              </div>
            </dl>
          </div>
          <div className="neon-plot">
            <AreaChart values={p.history.total} max={max} slots={HISTORY_SLOTS} id="neon-total" />
            <span className="axis top mono">{formatSpeed(max, language)}</span>
            <span className="axis bottom mono">{t("lastMinute")}</span>
          </div>
        </section>

        <div className="neon-table-wrap">
          {p.visible.length === 0 ? (
            <div className="neon-empty">
              <Icon name="download" size={36} />
              <h3>{p.downloads.length === 0 ? t("emptyTitle") : t("noMatch")}</h3>
              {p.downloads.length === 0 && <p>{t("emptyBody")}</p>}
            </div>
          ) : (
            <table className="neon-table">
              <thead>
                <tr>
                  <th className="c-name">{t("colName")}</th>
                  <th className="c-size">{t("colSize")}</th>
                  <th className="c-seg">{t("colSegments")}</th>
                  <th className="c-speed">{t("colSpeed")}</th>
                  <th className="c-spark" />
                  <th className="c-eta">{t("colEta")}</th>
                  <th className="c-state">{t("colStatus")}</th>
                </tr>
              </thead>
              <tbody>
                {p.visible.map((d) => (
                  <NeonRow key={d.id} d={d} {...p} />
                ))}
              </tbody>
            </table>
          )}
        </div>

        {detail && <NeonDetails d={detail} {...p} />}
      </div>
    </div>
  );
}

function RailButton(props: { icon: IconName; label: string; active?: boolean; dot?: "ok" | "bad"; onClick: () => void }) {
  return (
    <button type="button" className={`neon-rail-btn${props.active ? " on" : ""}`} title={props.label} aria-label={props.label} onClick={props.onClick}>
      <Icon name={props.icon} size={19} />
      {props.dot && <span className={`neon-status-dot ${props.dot}`} />}
    </button>
  );
}

function statusText(d: Download, t: ReturnType<typeof useI18n>["t"]) {
  if (d.stopping) return t("stopping");
  if (d.state === "downloading" && d.connections === 0 && d.speed === 0) return t("connecting");
  return t(`state.${d.state}` as TextKey);
}

function NeonRow({ d, selected, actions, history }: { d: Download } & ViewProps) {
  const { t, language } = useI18n();
  const isSelected = selected.includes(d.id);
  const value = percent(d.downloaded, d.size);
  const running = d.state === "downloading";
  return (
    <tr
      className={isSelected ? "sel" : ""}
      onMouseDown={(e) => {
        if (e.button !== 0) return;
        actions.select(d.id, e.shiftKey ? "range" : e.ctrlKey || e.metaKey ? "toggle" : "single");
      }}
      onDoubleClick={() => (running || d.state === "queued" ? actions.openProgress(d) : actions.primary(d))}
      onContextMenu={(e) => {
        e.preventDefault();
        if (!isSelected) actions.select(d.id, "single");
        actions.menu(e, d);
      }}
    >
      <td className="c-name">
        <div className="neon-name">
          <Icon name={categoryIcon[d.category]} size={15} />
          <span title={d.fileName}>{d.fileName}</span>
        </div>
      </td>
      <td className="c-size mono">{d.size == null ? "—" : formatBytes(d.size, language)}</td>
      <td className="c-seg">
        <SegmentBlocks fills={blocks(d)} className={d.state} />
        <div className="neon-sub mono">
          {value == null ? formatBytes(d.downloaded, language) : `${Math.floor(value)}%`}
          {running && d.connections > 0 && ` · ${t("connShort", { n: d.connections })}`}
        </div>
      </td>
      <td className={`c-speed mono${running ? "" : " dim"}`}>{running ? formatSpeed(d.speed, language) : "—"}</td>
      <td className="c-spark">{running && <Sparkline values={history.perDownload.get(d.id) ?? []} />}</td>
      <td className="c-eta mono">{running ? formatDuration(d.eta) : ""}</td>
      <td className="c-state" title={d.error ?? undefined}>
        <span className={`neon-dot ${d.state}`} />
        {statusText(d, t)}
      </td>
    </tr>
  );
}

function NeonDetails({ d, actions, history }: { d: Download } & ViewProps) {
  const { t, language } = useI18n();
  const running = d.state === "downloading";
  const speeds = [...(history.segments.get(d.id) ?? new Map<number, number>()).entries()].sort((a, b) => a[0] - b[0]);
  const fastest = Math.max(1, ...speeds.map(([, v]) => v));
  const primary: { icon: IconName; label: string } =
    running || d.state === "queued"
      ? { icon: "pause", label: t("pause") }
      : d.state === "completed"
        ? { icon: "open", label: t("open") }
        : d.state === "failed"
          ? { icon: "retry", label: t("actRetry") }
          : { icon: "play", label: t("resume") };

  return (
    <section className={`neon-details${running ? "" : " single"}`}>
      <div>
        <div className="neon-details-head">
          <h3 title={d.fileName}>{d.fileName}</h3>
          <div className="neon-details-actions">
            <button type="button" className="neon-ghost" onClick={() => actions.primary(d)}>
              <Icon name={primary.icon} size={14} /> {primary.label}
            </button>
            {running && (
              <button type="button" className="neon-ghost" title={t("viewProgress")} onClick={() => actions.openProgress(d)}>
                <Icon name="window" size={14} />
              </button>
            )}
            <button type="button" className="neon-ghost" title={t("showInFolder")} onClick={() => actions.showInFolder(d)}>
              <Icon name="folder" size={14} />
            </button>
            <button type="button" className="neon-ghost danger" title={t("delete")} onClick={() => actions.remove([d.id])}>
              <Icon name="trash" size={14} />
            </button>
          </div>
        </div>
        <SegmentMap d={d} />
        <dl className="neon-kv">
          <dt>{t("savedTo")}</dt>
          <dd title={d.path}>{d.path}</dd>
          <dt>{t("resumable")}</dt>
          <dd>{d.resumable ? t("yes") : t("no")}</dd>
          <dt>{t("server")}</dt>
          <dd>{hostOf(d.url)}</dd>
          <dt>{t("downloadedLabel")}</dt>
          <dd className="mono">
            {formatBytes(d.downloaded, language)}
            {d.size != null && ` / ${formatBytes(d.size, language)}`}
          </dd>
          {d.error && (
            <>
              <dt>{t("error")}</dt>
              <dd className="error">{d.error}</dd>
            </>
          )}
          {running && d.eta != null && (
            <>
              <dt>{t("timeLeft")}</dt>
              <dd>{formatHuman(d.eta, language)}</dd>
            </>
          )}
        </dl>
      </div>
      {running && (
        <div>
          <h3>{t("connectionsTitle", { n: d.connections })}</h3>
          <div className="neon-conns">
            {speeds.length === 0 && <span className="dim">{t("connecting")}</span>}
            {speeds.map(([start, speed], i) => (
              <div key={start} className="neon-conn mono">
                <span>{t("segmentN", { n: i + 1 })}</span>
                <span className="bar">
                  <b style={{ width: `${(speed / fastest) * 100}%` }} />
                </span>
                <span>{formatSpeed(speed, language)}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  );
}

/** The real segments of the file, each filled as far as it got. */
export function SegmentMap({ d }: { d: Download }) {
  if (!d.size || d.segments.length === 0) {
    return <SegmentBlocks fills={blocks(d, 8)} className={`big ${d.state}`} />;
  }
  const size = d.size;
  return (
    <div className={`segment-map ${d.state}`}>
      {d.segments.map((s) => (
        <i key={s.start} style={{ left: `${(s.start / size) * 100}%`, width: `${((s.end - s.start) / size) * 100}%` }}>
          <b style={{ width: `${Math.min(100, (s.downloaded / Math.max(1, s.end - s.start)) * 100)}%` }} />
        </i>
      ))}
    </div>
  );
}
