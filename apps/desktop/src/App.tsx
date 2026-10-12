import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  type AddPrompt,
  type AppInfo,
  type Download,
  type LinksPrompt,
  type MediaSource,
  type PowerAction,
  type PowerCountdown,
  type Queue,
  type Settings,
  type TorrentSource,
  api,
} from "./api";
import { useAppearance } from "./appearance";
import { AddDialog } from "./components/AddDialog";
import { AddressDialog } from "./components/AddressDialog";
import { BatchDialog } from "./components/BatchDialog";
import { BrowsersDialog } from "./components/BrowsersDialog";
import { ContextMenu } from "./components/ContextMenu";
import { DeleteDialog } from "./components/DeleteDialog";
import { DownloadDetails } from "./components/DownloadDetails";
import { MediaDialog } from "./components/MediaDialog";
import { PowerDialog } from "./components/PowerDialog";
import { SchedulerDialog } from "./components/SchedulerDialog";
import { SettingsDialog } from "./components/SettingsDialog";
import { TorrentDialog } from "./components/TorrentDialog";
import { type Toast, Toasts } from "./components/Toasts";
import { UpdateNotice } from "./components/UpdateNotice";
import { I18nContext, copyText, errorText } from "./context";
import { type Filter, type History, type TypeFilter, emptyHistory, matches, matchesType, recordHistory, totals } from "./model";
import { MinimalView } from "./skins/Minimal";
import { NeonView } from "./skins/Neon";
import type { RowActions, ViewProps } from "./views";

type DialogState =
  | { kind: "add"; prompt?: AddPrompt; url?: string }
  | { kind: "media"; source: MediaSource }
  | { kind: "torrent"; source: TorrentSource }
  | { kind: "settings" }
  | { kind: "browsers" }
  | { kind: "scheduler" }
  | { kind: "batch"; page?: LinksPrompt }
  | { kind: "address"; id: number }
  | { kind: "delete"; ids: number[] }
  | { kind: "details"; id: number };

interface Menu {
  x: number;
  y: number;
  download: Download;
}

const POLL_MS = 600;

const isMagnet = (text: string) => /^magnet:\?\S+$/i.test(text.trim());

/** Opens magnet links in the torrent dialog, other links in the add dialog. */
function dialogFor(url: string): DialogState {
  return isMagnet(url) ? { kind: "torrent", source: { link: url.trim() } } : { kind: "add", url };
}

export default function App() {
  const [settings, setSettings] = useState<Settings | null>(null);
  // Settings being tried out in the settings dialog, so a new look shows right away.
  const [preview, setPreview] = useState<Settings | null>(null);
  const [downloads, setDownloads] = useState<Download[]>([]);
  const [history, setHistory] = useState<History>(emptyHistory);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [typeFilter, setTypeFilter] = useState<TypeFilter>("all");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<number[]>([]);
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [menu, setMenu] = useState<Menu | null>(null);
  const [prompts, setPrompts] = useState<(AddPrompt | MediaSource | TorrentSource | LinksPrompt)[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [queues, setQueues] = useState<Queue[]>([]);
  const [queueFilter, setQueueFilter] = useState("all");
  const [afterAll, setAfterAllState] = useState<PowerAction>("nothing");
  const [countdown, setCountdown] = useState<PowerCountdown | null>(null);
  const anchor = useRef<number | null>(null);

  const { i18n, skin, dark } = useAppearance(preview ?? settings);
  const { t } = i18n;

  const toast = useCallback((kind: Toast["kind"], text: string, url?: string) => {
    const id = Date.now() + Math.random();
    setToasts((list) => [...list.filter((item) => !(url && item.url === url)), { id, kind, text, url }]);
    setTimeout(() => setToasts((list) => list.filter((item) => item.id !== id)), kind === "clipboard" ? 15000 : 7000);
  }, []);
  const showError = useCallback((message: string) => toast("error", message), [toast]);

  // Load settings and keep the list and the speed history fresh.
  useEffect(() => {
    api.getSettings().then(setSettings).catch((e) => showError(errorText(e)));
    let alive = true;
    let refreshing = false;
    const refresh = async () => {
      if (refreshing) return;
      refreshing = true;
      try {
        const list = await api.listDownloads();
        if (!alive) return;
        setDownloads(list);
        setHistory((before) => recordHistory(before, list));
      } catch (error) {
        console.error(error);
      } finally {
        refreshing = false;
      }
    };
    const refreshInfo = () => {
      api.appInfo().then((value) => alive && setInfo(value)).catch(() => {});
      // Schedules turn themselves off and "when all are done" resets.
      api.listQueues().then((value) => alive && setQueues(value)).catch(() => {});
      api.getAfterAll().then((value) => alive && setAfterAllState(value)).catch(() => {});
    };
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
      api.onMediaPrompt((source) => setPrompts((queue) => [...queue, source])),
      api.onLinksPrompt((page) => setPrompts((queue) => [...queue, page])),
      api.onTorrentPrompt((source) => setPrompts((queue) => [...queue, source])),
      api.takePendingTorrents().then((sources) => {
        setPrompts((queue) => [...queue, ...sources]);
        return () => {};
      }),
      api.onFileDrop((paths) => {
        const torrents = paths.filter((path) => /\.torrent$/i.test(path)).map((path) => ({ path }));
        setPrompts((queue) => [...queue, ...torrents]);
      }),
      api.onClipboardLink((url) => toast("clipboard", t("clipboardFound"), url)),
      api.onPowerCountdown(setCountdown),
      api.powerCountdown().then((value) => {
        if (value) setCountdown(value);
        return () => {};
      }),
    ];
    return () => stops.forEach((stop) => stop.then((unlisten) => unlisten()));
  }, [toast, t]);

  // Show browser downloads one after another.
  useEffect(() => {
    if (!dialog && prompts.length > 0 && settings) {
      const next = prompts[0];
      setDialog(
        "links" in next
          ? { kind: "batch", page: next }
          : "pageUrl" in next
          ? { kind: "media", source: next }
          : "url" in next
            ? { kind: "add", prompt: next }
            : { kind: "torrent", source: next },
      );
      setPrompts((queue) => queue.slice(1));
    }
  }, [dialog, prompts, settings]);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return downloads
      .filter((d) => matches(filter, d))
      .filter((d) => matchesType(typeFilter, d))
      .filter((d) => queueFilter === "all" || d.queue === queueFilter)
      .filter((d) => !q || d.fileName.toLowerCase().includes(q) || d.url.toLowerCase().includes(q))
      .sort((a, b) => b.addedAt - a.addedAt);
  }, [downloads, filter, typeFilter, queueFilter, query]);

  // A deleted queue can no longer be shown.
  useEffect(() => {
    if (queueFilter !== "all" && queues.length > 0 && !queues.some((q) => q.id === queueFilter)) setQueueFilter("all");
  }, [queues, queueFilter]);

  const reloadQueues = useCallback(() => {
    api.listQueues().then(setQueues).catch(() => {});
  }, []);

  // Bulk actions apply only to rows the user can see in the current filter.
  useEffect(() => {
    setSelected((ids) => {
      const kept = ids.filter((id) => visible.some((d) => d.id === id));
      return kept.length === ids.length ? ids : kept;
    });
  }, [visible]);

  const run = useCallback(
    (action: Promise<unknown>) => action.catch((error) => showError(errorText(error))),
    [showError],
  );

  const actions: RowActions = {
    selectAll: setSelected,
    details: (d) => setDialog({ kind: "details", id: d.id }),
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
    moveToQueue: (ids, queue) => run(api.moveToQueue(ids, queue)),
    newAddress: (d) => setDialog({ kind: "address", id: d.id }),
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
      if (/^https?:\/\/\S+$/i.test(text) || isMagnet(text)) setDialog(dialogFor(text));
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
    typeFilter,
    setTypeFilter,
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
    onSettings: () => {
      // Windows Task Manager may have changed startup since SDM opened.
      // Refresh before creating the draft so saving another preference does
      // not silently re-enable an externally disabled startup entry.
      void api.getSettings().then((current) => {
        setSettings(current);
        setDialog({ kind: "settings" });
      }).catch((error) => showError(errorText(error)));
    },
    onBrowsers: () => setDialog({ kind: "browsers" }),
    queues,
    queueFilter,
    setQueueFilter,
    afterAll,
    setAfterAll: (action) => {
      setAfterAllState(action);
      run(api.setAfterAll(action));
    },
    onScheduler: () => setDialog({ kind: "scheduler" }),
    onStartQueue: (queue) => run(api.startQueue(queue)),
    onStopQueue: (queue) => run(api.stopQueue(queue)),
  };

  return (
    <I18nContext.Provider value={i18n}>
      {view && (skin === "neon" ? <NeonView {...view} /> : <MinimalView {...view} dark={dark} />)}
      {view && <UpdateNotice />}

      {dialog?.kind === "details" && view && (
        <DownloadDetails d={downloads.find((d) => d.id === dialog.id)} history={history} actions={actions} onClose={closeDialog} />
      )}

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          download={downloads.find((d) => d.id === menu.download.id) ?? menu.download}
          selected={selected}
          queues={queues}
          actions={actions}
          onClose={closeMenu}
        />
      )}

      {dialog?.kind === "add" && settings && (
        <AddDialog
          prompt={dialog.prompt}
          initialUrl={dialog.url}
          settings={settings}
          queues={queues}
          onClose={closeDialog}
          onError={showError}
          onMedia={(pageUrl) => setDialog({ kind: "media", source: { pageUrl } })}
          onTorrent={(source) => setDialog({ kind: "torrent", source })}
          onBatch={() => setDialog({ kind: "batch" })}
        />
      )}
      {dialog?.kind === "torrent" && settings && (
        <TorrentDialog
          source={dialog.source}
          settings={settings}
          onClose={closeDialog}
          onError={showError}
          onShowExisting={(id) => {
            setFilter("all");
            setTypeFilter("all");
            setQuery("");
            setSelected([id]);
          }}
        />
      )}
      {dialog?.kind === "media" && settings && (
        <MediaDialog
          source={dialog.source}
          settings={settings}
          onClose={closeDialog}
          onError={showError}
          onDirect={(url) =>
            setDialog({
              kind: "add",
              prompt: { url, fileName: null, headers: dialog.source.referrer ? [["Referer", dialog.source.referrer]] : [] },
            })
          }
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
      {dialog?.kind === "scheduler" && settings && (
        <SchedulerDialog
          queues={queues}
          downloads={downloads}
          settings={settings}
          onClose={closeDialog}
          onError={showError}
          onChanged={reloadQueues}
          onSettings={setSettings}
        />
      )}
      {dialog?.kind === "batch" && settings && (
        <BatchDialog page={dialog.page} settings={settings} queues={queues} onClose={closeDialog} onError={showError} />
      )}
      {dialog?.kind === "address" && downloads.some((d) => d.id === dialog.id) && (
        <AddressDialog
          download={downloads.find((d) => d.id === dialog.id)!}
          onClose={closeDialog}
          onDone={(text) => toast("info", text)}
        />
      )}
      {countdown && <PowerDialog countdown={countdown} onError={showError} />}
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
        onDownload={(url) => setDialog(dialogFor(url))}
      />
    </I18nContext.Provider>
  );
}
