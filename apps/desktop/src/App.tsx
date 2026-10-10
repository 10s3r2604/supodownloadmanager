import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { type AddPrompt, type AppInfo, type Download, type Settings, api } from "./api";
import { useAppearance } from "./appearance";
import { AddDialog } from "./components/AddDialog";
import { BrowsersDialog } from "./components/BrowsersDialog";
import { ContextMenu } from "./components/ContextMenu";
import { DeleteDialog } from "./components/DeleteDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { type Toast, Toasts } from "./components/Toasts";
import { I18nContext, copyText, errorText } from "./context";
import { type Filter, type History, emptyHistory, matches, recordHistory, totals } from "./model";
import { MinimalView } from "./skins/Minimal";
import { NeonView } from "./skins/Neon";
import type { RowActions, ViewProps } from "./views";

type DialogState =
  | { kind: "add"; prompt?: AddPrompt; url?: string }
  | { kind: "settings" }
  | { kind: "browsers" }
  | { kind: "delete"; ids: number[] };

interface Menu {
  x: number;
  y: number;
  download: Download;
}

const POLL_MS = 600;

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  // Settings being tried out in the settings dialog, so a new look shows right away.
  const [preview, setPreview] = useState<Settings | null>(null);
  const [downloads, setDownloads] = useState<Download[]>([]);
  const [history, setHistory] = useState<History>(emptyHistory);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<number[]>([]);
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [menu, setMenu] = useState<Menu | null>(null);
  const [prompts, setPrompts] = useState<AddPrompt[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const anchor = useRef<number | null>(null);

  const { i18n, skin, dark } = useAppearance(preview ?? settings);
  const { t } = i18n;

  const toast = useCallback((kind: Toast["kind"], text: string, url?: string) => {
    const id = Date.now() + Math.random();
    setToasts((list) => [...list.filter((item) => !(url && item.url === url)), { id, kind, text, url }]);
    setTimeout(() => setToasts((list) => list.filter((item) => item.id !== id)), kind === "error" ? 7000 : 15000);
  }, []);
  const showError = useCallback((message: string) => toast("error", message), [toast]);

  // Load settings and keep the list and the speed history fresh.
  useEffect(() => {
    api.getSettings().then(setSettings).catch((e) => showError(errorText(e)));
    let alive = true;
    const refresh = async () => {
      try {
        const list = await api.listDownloads();
        if (!alive) return;
        setDownloads(list);
        setHistory((before) => recordHistory(before, list));
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
    primary(d) {
      if (d.state === "downloading" || d.state === "queued") run(api.pause([d.id]));
      else if (d.state === "completed") run(api.open(d.id));
      else run(api.resume([d.id]));
    },
    open: (d) => run(api.open(d.id)),
    showInFolder: (d) => run(api.showInFolder(d.id)),
    pause: (ids) => run(api.pause(ids)),
    resume: (ids) => run(api.resume(ids)),
    copyLink: (d) => run(copyText(d.url)),
    remove: (ids) => setDialog({ kind: "delete", ids }),
    openProgress: (d) => run(api.openProgressWindow(d.id)),
    menu: (event, download) => setMenu({ x: event.clientX, y: event.clientY, download }),
  };

  // Keyboard shortcuts and pasting links.
  useEffect(() => {
    const typing = (target: EventTarget | null) =>
      target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenu(null);
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

  const closeDialog = useCallback(() => setDialog(null), []);
  const closeMenu = useCallback(() => setMenu(null), []);
  const shown = preview ?? settings;

  const view: ViewProps | null = shown && {
    downloads,
    visible,
    filter,
    setFilter,
    query,
    setQuery,
    selected,
    actions,
    settings: shown,
    info,
    history,
    totals: totals(downloads, shown),
    onAdd: () => setDialog({ kind: "add" }),
    onResumeAll: () => run(api.resumeAll()),
    onPauseAll: () => run(api.pauseAll()),
    onSettings: () => setDialog({ kind: "settings" }),
    onBrowsers: () => setDialog({ kind: "browsers" }),
  };

  return (
    <I18nContext.Provider value={i18n}>
      {view && (skin === "minimal" ? <MinimalView {...view} dark={dark} /> : <NeonView {...view} />)}

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          download={downloads.find((d) => d.id === menu.download.id) ?? menu.download}
          selected={selected}
          actions={actions}
          onClose={closeMenu}
        />
      )}

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
        <SettingsDialog
          settings={settings}
          onPreview={setPreview}
          onClose={() => {
            setPreview(null);
            closeDialog();
          }}
          onSaved={setSettings}
          onError={showError}
        />
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
    </I18nContext.Provider>
  );
}
