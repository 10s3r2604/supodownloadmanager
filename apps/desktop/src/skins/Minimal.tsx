import { useRef, useEffect, useState } from "react";

import type { Download } from "../api";
import { wordmark } from "../brand";
import { useI18n } from "../context";
import { formatBytes, formatHuman, formatSpeed, percent } from "../format";
import type { TextKey } from "../i18n";
import { Icon } from "../icons";
import {
  CATEGORIES,
  type Filter,
  type TypeFilter,
  hostOf,
  matches,
  matchesType,
} from "../model";
import type { ViewProps } from "../views";
import { AreaChart, niceMax } from "../components/Charts";
import "./minimal.css";

export function MinimalView(p: ViewProps & { dark: boolean }) {
  const { t, language } = useI18n();
  const [graph, setGraph] = useState(false);
  const count = (f: Filter) =>
    p.downloads.filter((d) => matchesType(p.typeFilter, d) && matches(f, d))
      .length;
  const tabs: Filter[] = ["all", "active", "completed", "attention"];
  const chosen = p.visible.filter((d) => p.selected.includes(d.id));
  const canPause = chosen.some(
    (d) => d.state === "downloading" || d.state === "queued",
  );
  const canResume = chosen.some(
    (d) => d.state === "paused" || d.state === "failed",
  );
  const hasPaused = p.downloads.some(
    (d) => d.state === "paused" || d.state === "failed",
  );
  const hasActive = p.downloads.some(
    (d) => d.state === "downloading" || d.state === "queued",
  );
  const all = p.visible.length > 0 && chosen.length === p.visible.length;
  const selectAll = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (selectAll.current)
      selectAll.current.indeterminate = chosen.length > 0 && !all;
  }, [all, chosen.length]);

  return (
    <main className="minimal">
      <header className="mn-head">
        <img
          src={wordmark(p.dark ? "#eef4ef" : "#20332a")}
          height={48}
          alt="supo"
        />
        <span className="mn-space" />
        <button type="button" className="mn-link" onClick={p.onBrowsers}>
          {t("browsers")}
          {p.info?.bridgeError && (
            <span
              className="mn-warn-dot"
              title={t("bridgeError", { error: p.info.bridgeError })}
            />
          )}
        </button>
        <button type="button" className="mn-link" onClick={p.onSettings}>
          {t("settings")}
        </button>
        <button
          type="button"
          className="button primary mn-add"
          onClick={p.onAdd}
          title={`${t("add")} (Ctrl+N)`}
        >
          <Icon name="plus" size={18} /> {t("add")}
        </button>
      </header>

      <section className="mn-hero">
        <h1>{t("downloadsTitle")}</h1>
        <p className="mn-lead">
          {p.totals.active > 0 ? (
            <>
              <span className="mn-live" />{" "}
              {t("downloadingCount", { n: p.totals.active })}
              {" · "}
              {formatSpeed(p.totals.speed, language)}
              {p.totals.remaining != null && (
                <>
                  {" "}
                  ·{" "}
                  {t("aboutRemaining", {
                    time: formatHuman(p.totals.remaining, language),
                  })}
                </>
              )}
            </>
          ) : p.downloads.length === 0 ? (
            t("emptyBody")
          ) : (
            t("leadIdle", {
              count: p.downloads.length,
              today: formatBytes(p.totals.downloadedToday, language),
            })
          )}
          {p.totals.speedLimit != null && (
            <>
              {" "}
              · {t("limit")}: {formatSpeed(p.totals.speedLimit, language)}
            </>
          )}
        </p>
      </section>

      <div className="mn-filters">
        <nav className="mn-tabs" aria-label={t("colStatus")}>
          {tabs.map((f) => (
            <button
              type="button"
              key={f}
              aria-current={p.filter === f ? "true" : undefined}
              className={p.filter === f ? "on" : ""}
              onClick={() => p.setFilter(f)}
            >
              {t(`tab.${f}` as TextKey)} <span>{count(f)}</span>
            </button>
          ))}
        </nav>
        <div className="mn-tools">
          <label className="mn-type">
            <select
              aria-label={t("categories")}
              value={p.typeFilter}
              onChange={(e) => p.setTypeFilter(e.target.value as TypeFilter)}
            >
              <option value="all">{t("allCategories")}</option>
              {CATEGORIES.map((c) => (
                <option key={c} value={c}>
                  {t(`cat.${c}` as TextKey)}
                </option>
              ))}
              <option value="torrent">{t("bittorrent")}</option>
            </select>
            <Icon name="chevron" size={14} />
          </label>
          <label className="mn-search">
            <Icon name="search" size={16} />
            <input
              type="search"
              aria-label={t("search")}
              value={p.query}
              placeholder={t("search")}
              onChange={(e) => p.setQuery(e.target.value)}
            />
          </label>
        </div>
      </div>

      <div className="mn-list-head">
        <label className="mn-select-all" title={t("selectAllVisible")}>
          <input
            ref={selectAll}
            type="checkbox"
            aria-label={t("selectAllVisible")}
            checked={all}
            disabled={!p.visible.length}
            onChange={() =>
              p.actions.selectAll(all ? [] : p.visible.map((d) => d.id))
            }
          />
          <span>
            {chosen.length
              ? t("selectedCount", { n: chosen.length })
              : t("yourDownloads")}
          </span>
        </label>
        <div className="mn-bulk">
          {chosen.length > 0 ? (
            <>
              {canPause && (
                <button
                  type="button"
                  className="mn-link"
                  onClick={() => p.actions.pause(p.selected)}
                >
                  {t("pause")}
                </button>
              )}
              {canResume && (
                <button
                  type="button"
                  className="mn-link"
                  onClick={() => p.actions.resume(p.selected)}
                >
                  {t("resume")}
                </button>
              )}
              <button
                type="button"
                className="mn-link"
                onClick={() => p.actions.remove(p.selected)}
              >
                {t("delete")}
              </button>
              <button
                type="button"
                className="mn-link"
                onClick={() => p.actions.selectAll([])}
              >
                {t("clearSelection")}
              </button>
            </>
          ) : (
            <>
              {hasActive && (
                <button
                  type="button"
                  className="mn-link"
                  onClick={p.onPauseAll}
                >
                  {t("pauseAll")}
                </button>
              )}
              {hasPaused && (
                <button
                  type="button"
                  className="mn-link"
                  onClick={p.onResumeAll}
                >
                  {t("resumeAll")}
                </button>
              )}
              <button
                type="button"
                className="mn-link"
                aria-expanded={graph}
                onClick={() => setGraph(!graph)}
              >
                {t(graph ? "hideSpeedGraph" : "showSpeedGraph")}
              </button>
            </>
          )}
        </div>
      </div>
      {graph && (
        <div
          className="mn-graph"
          role="img"
          aria-label={`${t("totalSpeed")}: ${formatSpeed(p.totals.speed, language)}`}
        >
          <AreaChart
            values={p.history.total}
            max={niceMax(Math.max(0, ...p.history.total))}
            slots={100}
            id="total-speed"
          />
          <span>
            {t("totalSpeed")} · {formatSpeed(p.totals.speed, language)}
          </span>
          <small>{t("lastMinute")}</small>
        </div>
      )}
      <div className="mn-list" aria-label={t("downloadsTitle")}>
        {p.visible.length === 0 && (
          <div className="mn-empty">
            <Icon name="download" size={28} />
            <h2>{t(p.downloads.length === 0 ? "emptyTitle" : "noMatch")}</h2>
            {p.downloads.length === 0 && (
              <button
                type="button"
                className="button primary"
                onClick={p.onAdd}
              >
                {t("add")}
              </button>
            )}
          </div>
        )}
        {p.visible.map((d) => (
          <MinimalRow key={d.id} d={d} {...p} />
        ))}
      </div>
    </main>
  );
}

function MinimalRow({ d, selected, actions }: { d: Download } & ViewProps) {
  const { t, language } = useI18n();
  const isSelected = selected.includes(d.id);
  const value = d.state === "completed" ? 100 : percent(d.downloaded, d.size);
  const running = d.state === "downloading";
  const source =
    d.torrent && running
      ? t("peersShort", { n: d.connections })
      : hostOf(d.url);
  const status = d.stopping
    ? t("stopping")
    : d.seeding
      ? t("seeding")
      : running && d.checking
        ? t("checkingData")
        : running && d.processing
          ? t("processing")
          : running && d.eta != null
            ? t("timeLeftShort", { t: formatHuman(d.eta, language) })
            : running
              ? t("connecting")
              : t(`state.${d.state}`);
  const size =
    d.state === "completed"
      ? formatBytes(d.size ?? d.downloaded, language)
      : `${formatBytes(d.downloaded, language)} / ${d.size == null ? t("unknownSize") : formatBytes(d.size, language)}`;
  const ext = d.fileName.match(/\.([a-z0-9]{1,8})$/i)?.[1].toUpperCase();
  const action =
    running || d.state === "queued"
      ? t("actPause")
      : d.state === "completed"
        ? t("actOpen")
        : d.state === "failed"
          ? t("actRetry")
          : t("actResume");

  return (
    <div
      className={`mn-row ${d.state}${isSelected ? " sel" : ""}`}
      onMouseDown={(e) => {
        if (
          e.button !== 0 ||
          (e.target as HTMLElement).closest("button, input, label")
        )
          return;
        actions.select(
          d.id,
          e.shiftKey ? "range" : e.ctrlKey || e.metaKey ? "toggle" : "single",
        );
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        if (!isSelected) actions.select(d.id, "single");
        actions.menu(e, d);
      }}
    >
      <label
        className="mn-select"
        title={t("selectDownload", { name: d.fileName })}
      >
        <input
          type="checkbox"
          aria-label={t("selectDownload", { name: d.fileName })}
          checked={isSelected}
          onChange={() => actions.select(d.id, "toggle")}
        />
        <span aria-hidden="true">
          {d.torrent ? (
            <Icon name="magnet" size={18} />
          ) : (
            ext || <Icon name="file" size={18} />
          )}
        </span>
      </label>
      <div className="mn-main">
        <button
          type="button"
          className="mn-name"
          title={`${d.fileName} — ${t("pwDetails")}`}
          onClick={() => actions.details(d)}
        >
          {d.fileName}
        </button>
        <div className="mn-meta">
          <span className="mn-source" title={`${status} · ${source}`}>
            <span className={d.state === "failed" ? "mn-error" : ""}>
              {status}
            </span>{" "}
            · {source}
          </span>
          <span className="mn-size">{size}</span>
        </div>
        <div
          className={`mn-line${value == null && running ? " indeterminate" : ""}`}
          role="progressbar"
          aria-label={d.fileName}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={value ?? undefined}
          aria-valuetext={
            value == null ? t("unknownSize") : `${Math.floor(value)}%`
          }
        >
          <i style={{ width: `${value ?? 0}%` }} />
        </div>
      </div>
      <div className="mn-num">
        <b>{value == null ? "—" : `${Math.floor(value)}%`}</b>
        <span>
          {d.seeding
            ? `↑ ${formatSpeed(d.uploadSpeed, language)}`
            : running
              ? formatSpeed(d.speed, language)
              : "—"}
        </span>
      </div>
      <div className="mn-actions">
        <button
          type="button"
          className="mn-act"
          disabled={d.stopping}
          onClick={() => actions.primary(d)}
        >
          {action}
        </button>
        <button
          type="button"
          className="mn-more"
          aria-label={`${t("more")}: ${d.fileName}`}
          aria-haspopup="menu"
          onClick={(e) => {
            const box = e.currentTarget.getBoundingClientRect();
            actions.menu(
              { clientX: box.right - 220, clientY: box.bottom + 4 },
              d,
            );
          }}
        >
          <Icon name="more" size={18} />
        </button>
      </div>
    </div>
  );
}
