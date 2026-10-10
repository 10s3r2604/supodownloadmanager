// Minimal: calm and typographic. No boxes, just type, hairlines and thin progress lines.

import type { Download } from "../api";
import { wordmark } from "../brand";
import { useI18n } from "../context";
import { formatBytes, formatDuration, formatHuman, formatSpeed, percent } from "../format";
import type { TextKey } from "../i18n";
import { Icon, categoryIcon } from "../icons";
import { CATEGORIES, type Filter, type Group, groupOf, hostOf, matches } from "../model";
import type { ViewProps } from "../views";
import "./minimal.css";

const GROUPS: { key: Group; label: TextKey }[] = [
  { key: "active", label: "groupActive" },
  { key: "today", label: "groupToday" },
  { key: "yesterday", label: "groupYesterday" },
  { key: "older", label: "groupOlder" },
];

export function MinimalView(p: ViewProps & { dark: boolean }) {
  const { t, language } = useI18n();
  const count = (f: Filter) => p.downloads.filter((d) => matches(f, d)).length;
  const tabs: Filter[] = ["all", "active", "completed", "unfinished"];
  const categories = CATEGORIES.filter((c) => count(`cat:${c}`) > 0);
  const hasPaused = p.downloads.some((d) => d.state === "paused" || d.state === "failed");
  const now = new Date();

  return (
    <div className="minimal">
      <header className="mn-head">
        <img src={wordmark(p.dark ? "#fafaf7" : "#141413")} height={24} alt="supo" />
        <span className="mn-space" />
        <button type="button" className="mn-link strong" onClick={p.onAdd}>
          <Icon name="plus" size={16} /> {t("add")} <kbd>Ctrl N</kbd>
        </button>
        <button type="button" className="mn-link" onClick={p.onBrowsers}>
          <Icon name="globe" size={16} /> {t("browsers")}
          {p.info?.bridgeError && <span className="mn-warn-dot" title={t("bridgeError", { error: p.info.bridgeError })} />}
        </button>
        <button type="button" className="mn-link" onClick={p.onSettings}>
          <Icon name="settings" size={16} /> {t("settings")}
        </button>
      </header>

      <section className="mn-hero">
        <h1>
          {p.totals.active > 0 && <span className="mn-live" />}
          {p.totals.active > 0 ? t("headingActive", { n: p.totals.active }) : p.downloads.length === 0 ? t("emptyTitle") : t("headingIdle")}
        </h1>
        <p className="mn-lead">
          {p.totals.active > 0 ? (
            <>
              <b>{formatSpeed(p.totals.speed, language)}</b>
              {" · "}
              {t("leadActive", {
                time: formatHuman(p.totals.remaining, language) || "—",
                today: formatBytes(p.totals.downloadedToday, language),
              })}
            </>
          ) : p.downloads.length === 0 ? (
            t("emptyBody")
          ) : (
            t("leadIdle", { count: p.downloads.length, today: formatBytes(p.totals.downloadedToday, language) })
          )}
        </p>
        {(p.totals.active > 0 || hasPaused) && (
          <div className="mn-bulk">
            {p.totals.active > 0 && (
              <button type="button" className="mn-link" onClick={p.onPauseAll}>
                {t("pauseAll")}
              </button>
            )}
            {hasPaused && (
              <button type="button" className="mn-link" onClick={p.onResumeAll}>
                {t("resumeAll")}
              </button>
            )}
          </div>
        )}
      </section>

      <nav className="mn-filters">
        {tabs.map((f) => (
          <button type="button" key={f} className={p.filter === f ? "on" : ""} onClick={() => p.setFilter(f)}>
            {t(`tab.${f}` as TextKey)}
            <i>{count(f)}</i>
          </button>
        ))}
        {categories.map((c) => (
          <button type="button" key={c} className={p.filter === `cat:${c}` ? "on" : ""} onClick={() => p.setFilter(`cat:${c}`)}>
            {t(`cat.${c}` as TextKey)}
          </button>
        ))}
        <label className="mn-search">
          <Icon name="search" size={15} />
          <input value={p.query} placeholder={t("search")} onChange={(e) => p.setQuery(e.target.value)} />
        </label>
      </nav>

      <div className="mn-list">
        {p.visible.length === 0 && p.downloads.length > 0 && <p className="mn-empty">{t("noMatch")}</p>}
        {GROUPS.map(({ key, label }) => {
          const items = p.visible.filter((d) => groupOf(d, now) === key);
          if (items.length === 0) return null;
          return (
            <section key={key}>
              <h2>{t(label)}</h2>
              {items.map((d) => (
                <MinimalRow key={d.id} d={d} {...p} />
              ))}
            </section>
          );
        })}
      </div>
    </div>
  );
}

function MinimalRow({ d, selected, actions }: { d: Download } & ViewProps) {
  const { t, language } = useI18n();
  const isSelected = selected.includes(d.id);
  const value = percent(d.downloaded, d.size);
  const running = d.state === "downloading";

  // Torrents show their peers where files show their server.
  const source = d.torrent && running && d.connections > 0 ? t("peersShort", { n: d.connections }) : hostOf(d.url);
  let meta: string;
  if (d.stopping) meta = t("stopping");
  else if (d.seeding) meta = `${t("seeding")} · ↑ ${formatSpeed(d.uploadSpeed, language)}`;
  else if (running && d.checking) meta = `${t("checkingData")} · ${hostOf(d.url)}`;
  else if (running && d.processing) meta = `${t("processing")} · ${hostOf(d.url)}`;
  else if (running) meta = d.eta != null ? `${t("timeLeftShort", { t: formatDuration(d.eta) })} · ${source}` : `${t("connecting")} · ${source}`;
  else if (d.state === "queued") meta = `${t("state.queued")} · ${hostOf(d.url)}`;
  else meta = hostOf(d.url);

  const action = running || d.state === "queued" ? t("actPause") : d.state === "completed" ? t("actOpen") : d.state === "failed" ? t("actRetry") : t("actResume");

  return (
    <div
      className={`mn-row ${d.state}${isSelected ? " sel" : ""}`}
      onMouseDown={(e) => {
        if (e.button !== 0 || (e.target as HTMLElement).closest("button")) return;
        actions.select(d.id, e.shiftKey ? "range" : e.ctrlKey || e.metaKey ? "toggle" : "single");
      }}
      onDoubleClick={() => (running || d.state === "queued" ? actions.openProgress(d) : actions.primary(d))}
      onContextMenu={(e) => {
        e.preventDefault();
        if (!isSelected) actions.select(d.id, "single");
        actions.menu(e, d);
      }}
    >
      <Icon name={d.torrent ? "magnet" : categoryIcon[d.category]} size={18} />
      <div className="mn-main">
        <div className="mn-name" title={d.fileName}>
          {d.fileName}
        </div>
        <div className="mn-meta">
          {meta}
          {d.error && <span className="mn-error"> · {d.error}</span>}
        </div>
        {d.state !== "completed" && (
          <div className="mn-line">
            <i style={{ width: `${value ?? 0}%` }} />
          </div>
        )}
      </div>
      <div className="mn-num">
        {d.state === "completed" ? (
          formatBytes(d.size, language)
        ) : (
          <>
            <b>{value == null ? "—" : `${Math.floor(value)}%`}</b>
            {running ? formatSpeed(d.speed, language) : d.state === "paused" ? t("pausedLower") : d.state === "failed" ? t("failedLower") : ""}
          </>
        )}
      </div>
      <div className="mn-actions">
        <button type="button" className="mn-act" onClick={() => actions.primary(d)}>
          {action}
        </button>
        <button
          type="button"
          className="mn-more"
          aria-label={t("more")}
          onClick={(e) => {
            const box = (e.currentTarget as HTMLElement).getBoundingClientRect();
            actions.menu({ clientX: box.left - 180, clientY: box.bottom + 4 }, d);
          }}
        >
          <Icon name="more" size={16} />
        </button>
      </div>
    </div>
  );
}
