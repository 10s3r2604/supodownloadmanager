import { useEffect, useRef, useState } from "react";

import type { Download, PowerAction } from "../api";
import { AreaChart, niceMax } from "../components/Charts";
import { POWER_ACTIONS, queueName } from "../components/SchedulerDialog";
import { useI18n } from "../context";
import { formatBytes, formatHuman, formatSpeed, percent } from "../format";
import type { TextKey } from "../i18n";
import { Icon, categoryIcon, type IconName } from "../icons";
import { CATEGORIES, type Filter, hostOf, matches, matchesType } from "../model";
import type { ViewProps } from "../views";
import "./minimal.css";

/** Compact desktop layout, retaining the saved `minimal` appearance preference. */
export function MinimalView(p: ViewProps & { dark: boolean }) {
  const { t, language } = useI18n();
  const [graph, setGraph] = useState(false);
  const [menu, setMenu] = useState<string | null>(null);
  const menuBar = useRef<HTMLElement>(null);
  const chosen = p.visible.filter((d) => p.selected.includes(d.id));
  const pauseIds = chosen.filter((d) => d.state === "downloading" || d.state === "queued").map((d) => d.id);
  const resumeIds = chosen.filter((d) => d.state === "paused" || d.state === "failed").map((d) => d.id);
  const hasActive = p.downloads.some((d) => d.state === "downloading" || d.state === "queued");
  const hasPaused = p.downloads.some((d) => d.state === "paused" || d.state === "failed");
  const all = p.visible.length > 0 && chosen.length === p.visible.length;
  const selectAll = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (selectAll.current) selectAll.current.indeterminate = chosen.length > 0 && !all;
  }, [all, chosen.length]);
  useEffect(() => {
    const outside = (event: MouseEvent) => { if (!menuBar.current?.contains(event.target as Node)) setMenu(null); };
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape") setMenu(null); };
    window.addEventListener("mousedown", outside);
    window.addEventListener("keydown", escape);
    return () => { window.removeEventListener("mousedown", outside); window.removeEventListener("keydown", escape); };
  }, []);
  const count = (filter: Filter) => p.downloads.filter((d) => matches(filter, d)).length;
  const chooseFilter = (filter: Filter) => { p.setFilter(filter); p.setTypeFilter("all"); p.setQueueFilter("all"); };
  const selectVisible = () => p.actions.selectAll(all ? [] : p.visible.map((d) => d.id));
  const runMenu = (action: () => void) => { setMenu(null); action(); };
  const resume = () => chosen.length ? p.actions.resume(resumeIds) : p.onResumeAll();
  const pause = () => chosen.length ? p.actions.pause(pauseIds) : p.onPauseAll();
  const currentQueue = p.queues.find((q) => q.id === p.queueFilter);
  const heading = currentQueue ? queueName(currentQueue, t) : p.typeFilter === "torrent" ? t("torrent") : p.typeFilter !== "all" ? t(`cat.${p.typeFilter}`) : t(`tab.${p.filter}` as TextKey);
  const tool = (icon: IconName, label: string, action: () => void, disabled = false, primary = false) => (
    <button type="button" className={`idm-tool${primary ? " primary-tool" : ""}`} disabled={disabled} onClick={action} title={label}><Icon name={icon} size={24} /><span>{label}</span></button>
  );
  const menuItem = (label: string, action: () => void, disabled = false) => (
    <button type="button" role="menuitem" disabled={disabled} onClick={() => runMenu(action)}>{label}</button>
  );

  return (
    <main className="minimal idm">
      <nav className="idm-menubar" ref={menuBar} aria-label={t("menuTools")}>
        {(["menuFile", "menuDownloads", "menuView", "menuTools"] as const).map((key) => (
          <div className="idm-menu" key={key}>
            <button type="button" aria-haspopup="menu" aria-expanded={menu === key} onClick={() => setMenu(menu === key ? null : key)}>{t(key)}</button>
            {menu === key && <div className="idm-menu-items" role="menu" aria-label={t(key)}>
              {key === "menuFile" && <>{menuItem(`${t("add")} · Ctrl+N`, p.onAdd)}{menuItem(t("scheduler"), p.onScheduler)}</>}
              {key === "menuDownloads" && <>{menuItem(t("resumeAll"), p.onResumeAll, !hasPaused)}{menuItem(t("pauseAll"), p.onPauseAll, !hasActive)}{menuItem(t("delete"), () => p.actions.remove(p.selected), !chosen.length)}</>}
              {key === "menuView" && <>{menuItem(t("selectAllVisible"), selectVisible, !p.visible.length)}{menuItem(t("clearSelection"), () => p.actions.selectAll([]), !chosen.length)}{menuItem(t(graph ? "hideSpeedGraph" : "showSpeedGraph"), () => setGraph(!graph))}</>}
              {key === "menuTools" && <>{menuItem(t("settings"), p.onSettings)}{menuItem(t("browsers"), p.onBrowsers)}{menuItem(t("scheduler"), p.onScheduler)}</>}
            </div>}
          </div>
        ))}
        <button type="button" className={`idm-bridge${p.info?.bridgeError ? " warning" : ""}`} onClick={p.onBrowsers} title={p.info?.bridgeError ?? t("browsers")}><i /><span>{t("browsers")}</span></button>
      </nav>
      <header className="idm-toolbar" aria-label={t("downloadsTitle")}>
        {tool("plus", t("add"), p.onAdd, false, true)}
        {tool("play", t("resume"), resume, chosen.length ? !resumeIds.length : !hasPaused)}
        {tool("pause", t("pause"), pause, chosen.length ? !pauseIds.length : !hasActive)}
        {tool("trash", t("delete"), () => p.actions.remove(p.selected), !chosen.length)}
        <span className="idm-tool-divider" />
        {tool("layers", t("queue"), p.onScheduler)}
        {tool("clock", t("scheduler"), p.onScheduler)}
        {tool("settings", t("settings"), p.onSettings)}
        {tool("globe", t("browsers"), p.onBrowsers)}
        <label className="idm-search"><Icon name="search" size={13} /><input type="search" aria-label={t("search")} placeholder={t("search")} value={p.query} onChange={(event) => p.setQuery(event.target.value)} /></label>
      </header>
      <div className="idm-workspace">
        <aside className="idm-sidebar" aria-label={t("categories")}>
          <h2>{t("downloadsTitle")}</h2>
          {(["all", "active", "completed", "attention", "unfinished"] as Filter[]).map((filter) => (
            <button type="button" key={filter} className={`idm-side-item${p.filter === filter && p.typeFilter === "all" && p.queueFilter === "all" ? " on" : ""}`} onClick={() => chooseFilter(filter)}>
              <Icon name={filter === "all" ? "list" : filter === "active" ? "download" : filter === "completed" ? "tick" : filter === "attention" ? "alert" : "pause"} size={15} /><span>{t(`tab.${filter}` as TextKey)}</span><small>{count(filter)}</small>
            </button>
          ))}
          <h2>{t("categories")}</h2>
          {CATEGORIES.map((category) => <button type="button" key={category} className={`idm-side-item${p.typeFilter === category ? " on" : ""}`} onClick={() => { p.setTypeFilter(category); p.setFilter("all"); p.setQueueFilter("all"); }}>
            <Icon name={categoryIcon[category]} size={15} /><span>{t(`cat.${category}`)}</span><small>{p.downloads.filter((d) => matchesType(category, d)).length}</small>
          </button>)}
          <button type="button" className={`idm-side-item${p.typeFilter === "torrent" ? " on" : ""}`} onClick={() => { p.setTypeFilter("torrent"); p.setFilter("all"); p.setQueueFilter("all"); }}><Icon name="magnet" size={15} /><span>{t("torrent")}</span><small>{p.downloads.filter((d) => d.torrent).length}</small></button>
          <h2>{t("queue")}</h2>
          {p.queues.map((queue) => <button type="button" key={queue.id} className={`idm-side-item${p.queueFilter === queue.id ? " on" : ""}`} onClick={() => { p.setQueueFilter(queue.id); p.setFilter("all"); p.setTypeFilter("all"); }}><Icon name="layers" size={15} /><span>{queueName(queue, t)}</span><small>{p.downloads.filter((d) => d.queue === queue.id).length}</small></button>)}
        </aside>
        <section className="idm-content" aria-label={t("downloadsTitle")}>
          <div className="idm-viewbar"><strong title={heading}>{heading}</strong><span>{chosen.length ? t("selectedCount", { n: chosen.length }) : p.visible.length}</span>
            <div className="idm-view-actions">
              {p.queueFilter !== "all" && <><button type="button" className="idm-small-button" onClick={() => p.onStartQueue(p.queueFilter)} title={t("startQueue")}><Icon name="play" size={12} />{t("startQueue")}</button><button type="button" className="idm-small-button" onClick={() => p.onStopQueue(p.queueFilter)} title={t("stopQueue")}><Icon name="pause" size={12} />{t("stopQueue")}</button></>}
              <button type="button" className="idm-small-button" aria-expanded={graph} onClick={() => setGraph(!graph)} title={t(graph ? "hideSpeedGraph" : "showSpeedGraph")}><Icon name="sliders" size={12} /><span>{t(graph ? "hideSpeedGraph" : "showSpeedGraph")}</span></button>
            </div>
          </div>
          {graph && <div className="idm-graph" role="img" aria-label={`${t("totalSpeed")}: ${formatSpeed(p.totals.speed, language)}`}><AreaChart values={p.history.total} max={niceMax(Math.max(0, ...p.history.total))} slots={100} id="total-speed" /><span>{t("totalSpeed")} · {formatSpeed(p.totals.speed, language)}</span><small>{t("lastMinute")}</small></div>}
          <div className="idm-table-scroll">
            <table className="idm-table" aria-label={t("downloadsTitle")}>
              <colgroup><col className="idm-check-col" /><col /><col className="idm-size-col" /><col className="idm-state-col" /><col className="idm-speed-col" /><col className="idm-eta-col" /><col className="idm-action-col" /></colgroup>
              <thead><tr><th className="idm-check-cell"><input ref={selectAll} type="checkbox" checked={all} disabled={!p.visible.length} aria-label={t("selectAllVisible")} onChange={selectVisible} /></th><th>{t("colName")}</th><th>{t("colSize")}</th><th>{t("colStatus")}</th><th>{t("colSpeed")}</th><th>{t("colEta")}</th><th><span className="sr-only">{t("more")}</span></th></tr></thead>
              <tbody>{p.visible.map((download) => <CompactRow key={download.id} d={download} {...p} />)}</tbody>
            </table>
            {!p.visible.length && <div className="idm-empty"><Icon name="download" size={24} /><h2>{t(p.downloads.length ? "noMatch" : "emptyTitle")}</h2>{!p.downloads.length && <><p>{t("emptyBody")}</p><button type="button" className="button primary" onClick={p.onAdd}>{t("add")}</button></>}</div>}
          </div>
        </section>
      </div>
      <footer className="idm-statusbar"><span>{p.downloads.length} · {t("downloadsTitle")}</span><i /><span>{t("downloadingCount", { n: p.totals.active })}</span>
        {chosen.length > 0 && <button type="button" onClick={() => p.actions.selectAll([])} title={t("clearSelection")}>{t("selectedCount", { n: chosen.length })} <Icon name="x" size={11} /></button>}
        <div className="idm-status-right">
          {(hasActive || p.afterAll !== "nothing") && <label className={`idm-when${p.afterAll !== "nothing" ? " on" : ""}`} title={t("whenAllDone")}><Icon name="power" size={12} /><select aria-label={t("whenAllDone")} value={p.afterAll} onChange={(event) => p.setAfterAll(event.target.value as PowerAction)}>{POWER_ACTIONS.map((action) => <option key={action} value={action}>{action === "nothing" ? t("whenAllDone") : t(`power.${action}` as TextKey)}</option>)}</select></label>}
          <b title={p.totals.speedLimit == null ? t("totalSpeed") : `${t("limit")}: ${formatSpeed(p.totals.speedLimit, language)}`}>{formatSpeed(p.totals.speed, language)}</b>
          {p.totals.remaining != null && <span className="idm-total-eta">{formatHuman(p.totals.remaining, language)}</span>}
        </div>
      </footer>
    </main>
  );
}

function CompactRow({ d, selected, actions }: { d: Download } & ViewProps) {
  const { t, language } = useI18n();
  const isSelected = selected.includes(d.id);
  const value = d.state === "completed" ? 100 : percent(d.downloaded, d.size);
  const running = d.state === "downloading";
  const status = d.stopping ? t("stopping") : d.seeding ? t("seeding") : running && d.checking ? t("checkingData") : running && d.processing ? t("processing") : running && !d.speed ? t("connecting") : t(`state.${d.state}`);
  const size = d.size == null ? (d.downloaded ? formatBytes(d.downloaded, language) : t("unknownSize")) : formatBytes(d.size, language);
  const extension = d.fileName.match(/\.([a-z0-9]{1,8})$/i)?.[1].toUpperCase();
  const action = running || d.state === "queued" ? t("actPause") : d.state === "completed" ? t("actOpen") : d.state === "failed" ? t("actRetry") : t("actResume");
  const detail = `${d.fileName}\n${hostOf(d.url)}\n${formatBytes(d.downloaded, language)} / ${size}${d.error ? `\n${d.error}` : ""}`;
  return (
    <tr className={`${d.state}${isSelected ? " sel" : ""}`} aria-selected={isSelected}
      onMouseDown={(event) => { if (event.button !== 0 || (event.target as HTMLElement).closest("button, input, label")) return; actions.select(d.id, event.shiftKey ? "range" : event.ctrlKey || event.metaKey ? "toggle" : "single"); }}
      onDoubleClick={(event) => { if (!(event.target as HTMLElement).closest("button, input")) actions.details(d); }}
      onContextMenu={(event) => { event.preventDefault(); if (!isSelected) actions.select(d.id, "single"); actions.menu(event, d); }}>
      <td className="idm-check-cell"><input type="checkbox" checked={isSelected} aria-label={t("selectDownload", { name: d.fileName })} onChange={() => actions.select(d.id, "toggle")} /></td>
      <td title={detail}><div className="idm-file-cell"><span className="idm-extension" aria-hidden="true">{d.torrent ? <Icon name="magnet" size={12} /> : extension || <Icon name={categoryIcon[d.category]} size={12} />}</span><button type="button" className="idm-file-name" onClick={(event) => actions.select(d.id, event.shiftKey ? "range" : event.ctrlKey || event.metaKey ? "toggle" : "single")} onDoubleClick={() => actions.details(d)} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); actions.details(d); } }}>{d.fileName}</button></div></td>
      <td title={`${formatBytes(d.downloaded, language)} / ${size}`}>{size}</td>
      <td><button type="button" className={`idm-progress-status${d.error ? " has-error" : ""}`} onClick={() => actions.details(d)} title={d.error ?? status}>{value != null && <i style={{ width: `${Math.min(100, value)}%` }} />}<span>{d.error ? <Icon name="alert" size={11} /> : d.state === "completed" ? <Icon name="tick" size={11} /> : null}{value == null ? "" : `${Math.floor(value)}% · `}{d.error ?? status}</span></button></td>
      <td>{d.seeding ? `↑ ${formatSpeed(d.uploadSpeed, language)}` : running && !d.checking && !d.processing ? formatSpeed(d.speed, language) : "—"}</td>
      <td title={d.eta == null ? undefined : formatHuman(d.eta, language)}>{running && !d.checking && !d.processing && d.eta != null ? formatHuman(d.eta, language) : "—"}</td>
      <td className="idm-row-actions"><button type="button" className="idm-row-action" disabled={d.stopping} onClick={() => actions.primary(d)} aria-label={`${action}: ${d.fileName}`} title={action}><Icon name={running || d.state === "queued" ? "pause" : d.state === "completed" ? "open" : d.state === "failed" ? "retry" : "play"} size={13} /></button><button type="button" className="idm-row-action" aria-label={`${t("more")}: ${d.fileName}`} aria-haspopup="menu" onClick={(event) => { const box = event.currentTarget.getBoundingClientRect(); actions.menu({ clientX: box.right - 220, clientY: box.bottom + 4 }, d); }}><Icon name="more" size={13} /></button></td>
    </tr>
  );
}
