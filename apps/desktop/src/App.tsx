import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { type AddPrompt, type AppInfo, type Download, type Settings, api } from "./api";
import { AddDialog } from "./components/AddDialog";
import { BrowsersDialog } from "./components/BrowsersDialog";
import { DeleteDialog } from "./components/DeleteDialog";
import { DetailsPanel } from "./components/DetailsPanel";
import { DownloadTable, type RowActions } from "./components/DownloadTable";
import { SettingsDialog } from "./components/SettingsDialog";
import { type Filter, Sidebar, matches } from "./components/Sidebar";
import { type Toast, Toasts } from "./components/Toasts";
import { Toolbar } from "./components/Toolbar";
import { I18nContext, copyText, errorText } from "./context";
import { formatBytes } from "./format";
import { resolveLanguage, translator } from "./i18n";
import { Icon } from "./icons";

type DialogState =
  | { kind: "add"; prompt?: AddPrompt; url?: string }
  | { kind: "settings" }
  | { kind: "browsers" }
  | { kind: "delete"; ids: number[] };

const POLL_MS = 600;

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [downloads, setDownloads] = useState<Download[]>([]);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<number[]>([]);
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [prompts, setPrompts] = useState<AddPrompt[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const anchor = useRef<number | null>(null);

  const language = resolveLanguage(settings?.language);
  const i18n = useMemo(() => ({ t: translator(language), language }), [language]);
  const { t } = i18n;

  // Theme and document language.
  useEffect(() => {
    const root = document.documentElement;
    root.lang = language;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const theme = settings?.theme ?? "system";
      root.dataset.theme = theme === "system" ? (media.matches ? "dark" : "light") : theme;
    };
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [settings?.theme, language]);

  const toast = useCallback((kind: Toast["kind"], text: string, url?: string) => {
    const id = Date.now() + Math.random();
    setToasts((list) => [...list.filter((item) => !(url && item.url === url)), { id, kind, text, url }]);
    setTimeout(() => setToasts((list) => list.filter((item) => item.id !== id)), kind === "error" ? 7000 : 15000);
  }, []);
  const showError = useCallback((message: string) => toast("error", message), [toast]);

  // Load settings and keep the list fresh.
  useEffect(() => {
    api.getSettings().then(setSettings).catch((e) => showError(errorText(e)));
    let alive = true;
    const refresh = async () => {
      try {
        const list = await api.listDownloads();
        if (alive) setDownloads(list);
      } catch (error) {
        console.error(error);
      }
    };
    const refreshInfo = () => api.appInfo().then((value) => alive && setInfo(value)).catch(() => {});
    refresh();
    refreshInfo();
    const timers = [setInterval(refresh, POLL_MS), setInterval(refreshInfo, 5000)];
    return () => {
      alive = false;
      timers.forEach(clearInterval);
    };
  }, [showError]);

  // Downloads from the browser and links from the clipboard.
  useEffect(() => {
    const stops = [
      api.onAddPrompt((prompt) => setPrompts((queue) => [...queue, prompt])),
      api.onClipboardLink((url) => toast("clipboard", t("clipboardFound"), url)),
    ];
    return () => stops.forEach((stop) => stop.then((unlisten) => unlisten()));
  }, [toast, t]);

  // Show browser downloads one after another.
  useEffect(() => {
    if (!dialog && prompts.length > 0 && settings) {
      setDialog({ kind: "add", prompt: prompts[0] });
      setPrompts((queue) => queue.slice(1));
    }
  }, [dialog, prompts, settings]);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return downloads
      .filter((d) => matches(filter, d))
      .filter((d) => !q || d.fileName.toLowerCase().includes(q) || d.url.toLowerCase().includes(q))
      .sort((a, b) => b.addedAt - a.addedAt);
  }, [downloads, filter, query]);

  // Forget selected rows that are gone.
  useEffect(() => {
    setSelected((ids) => {
      const kept = ids.filter((id) => downloads.some((d) => d.id === id));
      return kept.length === ids.length ? ids : kept;
    });
  }, [downloads]);

  const selection = downloads.filter((d) => selected.includes(d.id));
  const canResume = selection.some((d) => d.state === "paused" || d.state === "failed");
  const canPause = selection.some((d) => d.state === "downloading" || d.state === "queued");

  const run = useCallback(
    (action: Promise<unknown>) => action.catch((error) => showError(errorText(error))),
    [showError],
  );

  const actions: RowActions = {
    select(id, mode) {
      if (mode === "toggle") {
        setSelected((ids) => (ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id]));
        anchor.current = id;
      } else if (mode === "range" && anchor.current != null) {
        const order = visible.map((d) => d.id);
        const [from, to] = [order.indexOf(anchor.current), order.indexOf(id)].sort((a, b) => a - b);
        setSelected(from < 0 ? [id] : order.slice(from, to + 1));
      } else {
        setSelected([id]);
        anchor.current = id;
      }
    },
    open: (d) => run(api.open(d.id)),
    showInFolder: (d) => run(api.showInFolder(d.id)),
    pause: (ids) => run(api.pause(ids)),
    resume: (ids) => run(api.resume(ids)),
    copyLink: (d) => run(copyText(d.url)),
    remove: (ids) => setDialog({ kind: "delete", ids }),
  };

  // Keyboard shortcuts and pasting links.
  useEffect(() => {
    const typing = (target: EventTarget | null) =>
      target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));
    const onKey = (event: KeyboardEvent) => {
      if (dialog || typing(event.target)) return;
      const mod = event.ctrlKey || event.metaKey;
      if (event.key === "Delete" && selected.length > 0) {
        setDialog({ kind: "delete", ids: selected });
      } else if (mod && event.key.toLowerCase() === "n") {
        event.preventDefault();
        setDialog({ kind: "add" });
      } else if (mod && event.key.toLowerCase() === "a") {
        event.preventDefault();
        setSelected(visible.map((d) => d.id));
      }
    };
    const onPaste = (event: ClipboardEvent) => {
      if (dialog || typing(event.target)) return;
      const text = event.clipboardData?.getData("text")?.trim() ?? "";
      if (/^https?:\/\/\S+$/i.test(text)) setDialog({ kind: "add", url: text });
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("paste", onPaste);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("paste", onPaste);
    };
  }, [dialog, selected, visible]);

  const active = downloads.filter((d) => d.state === "downloading");
  const totalSpeed = active.reduce((sum, d) => sum + d.speed, 0);
  const details = selection.length === 1 ? selection[0] : null;
  const closeDialog = useCallback(() => setDialog(null), []);

  return (
    <I18nContext.Provider value={i18n}>
      <div className="app">
        <Toolbar
          canResume={canResume}
          canPause={canPause}
          canDelete={selection.length > 0}
          query={query}
          onQuery={setQuery}
          onAdd={() => setDialog({ kind: "add" })}
          onResume={() => run(api.resume(selected))}
          onPause={() => run(api.pause(selected))}
          onDelete={() => setDialog({ kind: "delete", ids: selected })}
          onResumeAll={() => run(api.resumeAll())}
          onPauseAll={() => run(api.pauseAll())}
          onBrowsers={() => setDialog({ kind: "browsers" })}
          onSettings={() => setDialog({ kind: "settings" })}
        />
        <div className="main">
          <Sidebar filter={filter} downloads={downloads} onChange={setFilter} />
          <main className="content">
            <DownloadTable
              downloads={visible}
              selected={selected}
              actions={actions}
              empty={
                downloads.length === 0
                  ? { title: t("emptyTitle"), body: t("emptyBody") }
                  : { title: t("noMatch") }
              }
            />
            {details && (
              <DetailsPanel
                download={details}
                onOpen={() => actions.open(details)}
                onShowInFolder={() => actions.showInFolder(details)}
              />
            )}
          </main>
        </div>
        <footer className="statusbar">
          <span>
            <Icon name="download" size={14} /> {t("downloadingCount", { n: active.length })}
          </span>
          <span className="speed">{formatBytes(totalSpeed, language)}/s</span>
          {settings && settings.speedLimit > 0 && (
            <span>{t("speedLimitOn", { value: formatBytes(settings.speedLimit, language) })}</span>
          )}
          <span className="spacer" />
          {info &&
            (info.bridgeError ? (
              <span className="warning" title={info.bridgeError}>
                <Icon name="alert" size={14} /> {t("bridgeError", { error: info.bridgeError })}
              </span>
            ) : (
              <span className="muted">
                <Icon name="globe" size={14} /> {t("bridgeOk", { port: info.bridgePort })}
              </span>
            ))}
          {info && <span className="muted">v{info.version}</span>}
        </footer>

        {dialog?.kind === "add" && settings && (
          <AddDialog
            prompt={dialog.prompt}
            initialUrl={dialog.url}
            settings={settings}
            onClose={closeDialog}
            onError={showError}
          />
        )}
        {dialog?.kind === "settings" && settings && (
          <SettingsDialog settings={settings} onClose={closeDialog} onSaved={setSettings} onError={showError} />
        )}
        {dialog?.kind === "browsers" && <BrowsersDialog info={info} onClose={closeDialog} onError={showError} />}
        {dialog?.kind === "delete" && (
          <DeleteDialog
            count={dialog.ids.length}
            hasFinished={downloads.some((d) => dialog.ids.includes(d.id) && d.state === "completed")}
            onClose={closeDialog}
            onConfirm={(deleteFiles) => {
              run(api.remove(dialog.ids, deleteFiles));
              setDialog(null);
            }}
          />
        )}

        <Toasts
          toasts={toasts}
          onDismiss={(id) => setToasts((list) => list.filter((item) => item.id !== id))}
          onDownload={(url) => setDialog({ kind: "add", url })}
        />
      </div>
    </I18nContext.Provider>
  );
}
