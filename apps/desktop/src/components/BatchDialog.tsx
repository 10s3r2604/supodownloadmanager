// Adds many downloads at once: from a numbered pattern such as
// https://example.com/img[001-120].jpg, or from the links of a web page.

import { useEffect, useMemo, useState } from "react";

import { type BatchLink, type Category, type LinksPrompt, MAIN_QUEUE, type Queue, type Settings, api } from "../api";
import { errorText, useI18n } from "../context";
import type { TextKey } from "../i18n";
import { Icon, categoryIcon } from "../icons";
import { CATEGORIES, categoryOf, hostOf } from "../model";
import { Dialog } from "./Dialog";
import { queueName } from "./SchedulerDialog";

interface Props {
  /** The links of a page; without it the dialog asks for a pattern. */
  page?: LinksPrompt;
  settings: Settings;
  queues: Queue[];
  onClose: () => void;
  onError: (message: string) => void;
}

interface Row {
  link: BatchLink;
  name: string;
  category: Category;
}

/** The file name a link would get. */
function nameOf(url: string) {
  try {
    const last = new URL(url).pathname.split("/").filter(Boolean).pop() ?? "";
    return decodeURIComponent(last);
  } catch {
    return "";
  }
}

function rowsOf(links: BatchLink[]): Row[] {
  return links.map((link) => {
    const name = link.fileName || nameOf(link.url);
    return { link, name, category: name.includes(".") ? categoryOf(name) : "other" };
  });
}

export function BatchDialog({ page, settings, queues, onClose, onError }: Props) {
  const { t } = useI18n();
  const [pattern, setPattern] = useState("");
  const [patternError, setPatternError] = useState<string | null>(null);
  const [rows, setRows] = useState<Row[]>(() => (page ? rowsOf(page.links) : []));
  // Files are picked at first; links to other pages are not.
  const [picked, setPicked] = useState<Set<string>>(
    () => new Set(page ? rowsOf(page.links).filter((r) => r.category !== "other").map((r) => r.link.url) : []),
  );
  const [type, setType] = useState<Category | "all">("all");
  const [query, setQuery] = useState("");
  const [directory, setDirectory] = useState("");
  const [queue, setQueue] = useState(MAIN_QUEUE);
  const [busy, setBusy] = useState(false);

  // A pattern becomes its links as it is typed.
  useEffect(() => {
    if (page) return;
    const text = pattern.trim();
    if (!/^https?:\/\/\S+$/i.test(text)) {
      setRows([]);
      setPatternError(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const links = await api.expandBatch(text);
        if (cancelled) return;
        const next = rowsOf(links.map((url) => ({ url })));
        setRows(next);
        setPicked(new Set(links));
        setPatternError(null);
      } catch (error) {
        if (cancelled) return;
        setRows([]);
        setPatternError(errorText(error));
      }
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [pattern, page]);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return rows.filter(
      (r) =>
        (type === "all" || r.category === type) &&
        (!q || r.name.toLowerCase().includes(q) || r.link.url.toLowerCase().includes(q) || r.link.text?.toLowerCase().includes(q)),
    );
  }, [rows, type, query]);
  const counts = useMemo(() => {
    const map = new Map<Category, number>();
    for (const r of rows) map.set(r.category, (map.get(r.category) ?? 0) + 1);
    return map;
  }, [rows]);
  const chosen = rows.filter((r) => picked.has(r.link.url));
  const allShown = shown.length > 0 && shown.every((r) => picked.has(r.link.url));

  function toggle(urls: string[], on: boolean) {
    setPicked((before) => {
      const next = new Set(before);
      for (const url of urls) {
        if (on) next.add(url);
        else next.delete(url);
      }
      return next;
    });
  }

  async function browse() {
    const folder = await api.pickFolder();
    if (folder) setDirectory(folder);
  }

  async function submit(start: boolean) {
    if (chosen.length === 0 || busy) return;
    setBusy(true);
    try {
      await api.addBatch({
        links: chosen.map((r) => r.link),
        directory: directory.trim() || undefined,
        start,
        queue,
      });
      onClose();
    } catch (error) {
      onError(errorText(error));
      setBusy(false);
    }
  }

  return (
    <Dialog
      title={page ? t("linksTitle") : t("batchTitle")}
      onClose={onClose}
      wide
      footer={
        <>
          <span className="batch-count muted">{t("batchChosen", { n: chosen.length, total: rows.length })}</span>
          <button type="button" className="button" onClick={onClose}>
            {t("cancel")}
          </button>
          <button type="button" className="button" disabled={chosen.length === 0 || busy} onClick={() => submit(false)}>
            {t("downloadLater")}
          </button>
          <button type="button" className="button primary" disabled={chosen.length === 0 || busy} onClick={() => submit(true)}>
            <Icon name="download" size={16} />
            {t("downloadNow")}
          </button>
        </>
      }
    >
      <div className="form">
        {page ? (
          <p className="batch-page muted">
            <Icon name="globe" size={14} /> {page.title || hostOf(page.pageUrl)} · {hostOf(page.pageUrl)}
          </p>
        ) : (
          <label className="field">
            <span>{t("batchPattern")}</span>
            <input
              autoFocus
              value={pattern}
              spellCheck={false}
              placeholder="https://example.com/photos/img[001-120].jpg"
              onChange={(e) => setPattern(e.target.value)}
            />
            <small className={patternError ? "warning" : "muted"}>{patternError ?? t("batchPatternHint")}</small>
          </label>
        )}

        {rows.length > 0 && (
          <>
            <div className="batch-tools">
              <div className="batch-types" role="group" aria-label={t("categories")}>
                <button type="button" className={type === "all" ? "on" : ""} aria-pressed={type === "all"} onClick={() => setType("all")}>
                  {t("allCategories")} <span>{rows.length}</span>
                </button>
                {CATEGORIES.filter((c) => counts.has(c)).map((c) => (
                  <button type="button" key={c} className={type === c ? "on" : ""} aria-pressed={type === c} onClick={() => setType(c)}>
                    {t(`cat.${c}` as TextKey)} <span>{counts.get(c)}</span>
                  </button>
                ))}
              </div>
              {page && (
                <label className="batch-search">
                  <Icon name="search" size={14} />
                  <input type="search" aria-label={t("search")} placeholder={t("search")} value={query} onChange={(e) => setQuery(e.target.value)} />
                </label>
              )}
            </div>
            <div className="batch-list" role="group" aria-label={t("batchLinks")}>
              <label className="batch-row batch-all">
                <input type="checkbox" checked={allShown} onChange={() => toggle(shown.map((r) => r.link.url), !allShown)} />
                <span>{t("selectAllVisible")}</span>
              </label>
              {shown.slice(0, 1000).map((r) => (
                <label className="batch-row" key={r.link.url} title={r.link.url}>
                  <input type="checkbox" checked={picked.has(r.link.url)} onChange={(e) => toggle([r.link.url], e.target.checked)} />
                  <Icon name={categoryIcon[r.category]} size={15} />
                  <span className="batch-name">{r.name || r.link.url}</span>
                  <span className="batch-host muted">{r.link.text && r.link.text !== r.name ? r.link.text : hostOf(r.link.url)}</span>
                </label>
              ))}
              {shown.length > 1000 && <p className="muted batch-more">{t("batchMore", { n: shown.length - 1000 })}</p>}
            </div>
          </>
        )}

        <div className="field-grid">
          <div className="field">
            <span>{t("saveIn")}</span>
            <div className="input-row">
              <input
                value={directory}
                spellCheck={false}
                placeholder={settings.categoryFolders ? t("batchTypeFolders") : settings.downloadDir}
                onChange={(e) => setDirectory(e.target.value)}
              />
              <button type="button" className="button" onClick={browse} aria-label={t("browse")} title={t("browse")}>
                <Icon name="folder" size={16} />
              </button>
            </div>
          </div>
          <label className="field">
            <span>{t("queue")}</span>
            <select value={queue} onChange={(e) => setQueue(e.target.value)}>
              {queues.map((q) => (
                <option key={q.id} value={q.id}>
                  {queueName(q, t)}
                </option>
              ))}
            </select>
          </label>
        </div>
      </div>
    </Dialog>
  );
}
