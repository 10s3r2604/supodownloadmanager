// Shows what is in a torrent and lets the user pick the files, like the
// "add torrent" dialog of BitTorrent clients.

import { useCallback, useEffect, useMemo, useState } from "react";

import { type Settings, type TorrentFile, type TorrentInfo, type TorrentSource, api } from "../api";
import { errorText, useI18n } from "../context";
import { formatBytes } from "../format";
import { Icon, categoryIcon } from "../icons";
import { categoryOf } from "../model";
import { Dialog } from "./Dialog";

interface Props {
  source: TorrentSource;
  settings: Settings;
  onClose: () => void;
  onError: (message: string) => void;
  /** Selects the download that already has this torrent. */
  onShowExisting: (id: number) => void;
}

type Stage = { kind: "reading" } | { kind: "ready" } | { kind: "failed"; error: string };

/** A folder or file in the torrent. */
interface TreeNode {
  name: string;
  /** Path inside the torrent; folders end with "/". */
  key: string;
  size: number;
  /** Indexes of all files at or below this node. */
  indexes: number[];
  children: TreeNode[];
  file?: TorrentFile;
}

function buildTree(files: TorrentFile[]): TreeNode[] {
  const root: TreeNode = { name: "", key: "", size: 0, indexes: [], children: [] };
  for (const file of files) {
    const parts = file.path.split("/");
    let node = root;
    node.size += file.size;
    node.indexes.push(file.index);
    parts.forEach((part, depth) => {
      const leaf = depth === parts.length - 1;
      const key = parts.slice(0, depth + 1).join("/") + (leaf ? "" : "/");
      let child = node.children.find((c) => c.key === key);
      if (!child) {
        child = { name: part, key, size: 0, indexes: [], children: [], file: leaf ? file : undefined };
        node.children.push(child);
      }
      child.size += file.size;
      child.indexes.push(file.index);
      node = child;
    });
  }
  const sort = (nodes: TreeNode[]) => {
    nodes.sort((a, b) => Number(Boolean(a.file)) - Number(Boolean(b.file)) || a.name.localeCompare(b.name));
    nodes.forEach((n) => sort(n.children));
  };
  sort(root.children);
  return root.children;
}

export function TorrentDialog({ source, settings, onClose, onError, onShowExisting }: Props) {
  const { t, language } = useI18n();
  const [stage, setStage] = useState<Stage>({ kind: "reading" });
  const [info, setInfo] = useState<TorrentInfo | null>(null);
  const [chosen, setChosen] = useState<Set<number>>(new Set());
  const [closed, setClosed] = useState<Set<string>>(new Set());
  const [directory, setDirectory] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setStage({ kind: "reading" });
    try {
      const found = await api.inspectTorrent(source);
      setInfo(found);
      setChosen(new Set(found.files.map((f) => f.index)));
      setStage({ kind: "ready" });
    } catch (error) {
      const message = errorText(error);
      if (message !== "cancelled") setStage({ kind: "failed", error: message });
    }
  }, [source]);

  useEffect(() => {
    load();
    // Stop looking for a magnet link's peers when the dialog closes.
    return () => {
      api.cancelTorrentInspect().catch(() => {});
    };
  }, [load]);

  const tree = useMemo(() => (info ? buildTree(info.files) : []), [info]);
  const chosenSize = useMemo(
    () => (info ? info.files.filter((f) => chosen.has(f.index)).reduce((sum, f) => sum + f.size, 0) : 0),
    [info, chosen],
  );
  const folder = directory ?? info?.directory ?? settings.downloadDir;
  const ready = stage.kind === "ready" && info != null;
  const blocked = !ready || chosen.size === 0 || info?.existing != null || busy;

  function toggle(indexes: number[], on: boolean) {
    setChosen((before) => {
      const next = new Set(before);
      for (const i of indexes) {
        if (on) next.add(i);
        else next.delete(i);
      }
      return next;
    });
  }

  async function browse() {
    const picked = await api.pickFolder();
    if (picked) setDirectory(picked);
  }

  async function submit(start: boolean) {
    if (!info || blocked) return;
    setBusy(true);
    try {
      await api.addTorrent({
        infoHash: info.infoHash,
        files: chosen.size === info.files.length ? null : [...chosen].sort((a, b) => a - b),
        directory: directory ?? undefined,
        start,
      });
      onClose();
    } catch (error) {
      onError(errorText(error));
      setBusy(false);
    }
  }

  const renderNode = (node: TreeNode, depth: number) => {
    const all = node.indexes.every((i) => chosen.has(i));
    const some = !all && node.indexes.some((i) => chosen.has(i));
    const open = !closed.has(node.key);
    return (
      <div key={node.key}>
        <label className={`tor-row${node.file ? "" : " folder"}`} style={{ paddingLeft: 10 + depth * 18 }}>
          {node.file ? (
            <span className="tor-caret" />
          ) : (
            <button
              type="button"
              className={`tor-caret${open ? " open" : ""}`}
              aria-label={node.name}
              onClick={(e) => {
                e.preventDefault();
                setClosed((before) => {
                  const next = new Set(before);
                  if (open) next.add(node.key);
                  else next.delete(node.key);
                  return next;
                });
              }}
            >
              <Icon name="chevron" size={13} />
            </button>
          )}
          <input
            type="checkbox"
            checked={all}
            ref={(el) => {
              if (el) el.indeterminate = some;
            }}
            onChange={(e) => toggle(node.indexes, e.target.checked)}
          />
          <Icon name={node.file ? categoryIcon[categoryOf(node.name)] : "folder"} size={15} />
          <span className="tor-name" title={node.file ? node.file.path : node.name}>
            {node.name}
          </span>
          <span className="tor-size">{formatBytes(node.size, language)}</span>
        </label>
        {!node.file && open && node.children.map((child) => renderNode(child, depth + 1))}
      </div>
    );
  };

  return (
    <Dialog
      title={t("torrentTitle")}
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            {t("cancel")}
          </button>
          {ready && (
            <>
              <button type="button" className="button" disabled={blocked} onClick={() => submit(false)}>
                {t("downloadLater")}
              </button>
              <button type="button" className="button primary" disabled={blocked} onClick={() => submit(true)}>
                <Icon name="download" size={16} />
                {t("downloadNow")}
              </button>
            </>
          )}
        </>
      }
    >
      {stage.kind === "reading" ? (
        <div className="media-wait">
          <span className="spinner" />
          <div>
            <strong>{t("torrentReading")}</strong>
            <div className="muted">{t("torrentReadingHint")}</div>
          </div>
        </div>
      ) : stage.kind === "failed" ? (
        <div className="media-failed">
          <p className="warning">
            <Icon name="alert" size={16} /> {t("torrentFailed", { error: stage.error })}
          </p>
          <div className="media-failed-actions">
            <button type="button" className="button" onClick={load}>
              <Icon name="retry" size={15} /> {t("mediaRetry")}
            </button>
          </div>
        </div>
      ) : (
        info && (
          <form
            className="form"
            onSubmit={(event) => {
              event.preventDefault();
              submit(true);
            }}
          >
            <div className="media-head">
              <div className="tor-badge">
                <Icon name="magnet" size={26} />
              </div>
              <div className="media-about">
                <div className="media-title" title={info.name}>
                  {info.name}
                </div>
                <div className="muted">
                  {[
                    formatBytes(info.size, language),
                    info.files.length === 1 ? t("torrentOneFile") : t("torrentFiles", { n: info.files.length }),
                    info.private && t("torrentPrivate"),
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </div>
                {info.comment && (
                  <div className="tor-comment faint" title={info.comment}>
                    {info.comment}
                  </div>
                )}
              </div>
            </div>

            {info.existing != null && (
              <div className="video-banner">
                <Icon name="alert" size={18} />
                <span>{t("torrentExisting")}</span>
                <button
                  type="button"
                  className="button"
                  onClick={() => {
                    onShowExisting(info.existing as number);
                    onClose();
                  }}
                >
                  {t("torrentShowExisting")}
                </button>
              </div>
            )}

            <div className="tor-files">
              <div className="tor-toolbar">
                <span>
                  {t("torrentSelected", {
                    n: chosen.size,
                    total: info.files.length,
                    size: formatBytes(chosenSize, language),
                  })}
                </span>
                <span className="tor-pick">
                  <button type="button" className="link-button" onClick={() => toggle(info.files.map((f) => f.index), true)}>
                    {t("torrentSelectAll")}
                  </button>
                  <button type="button" className="link-button" onClick={() => toggle(info.files.map((f) => f.index), false)}>
                    {t("torrentSelectNone")}
                  </button>
                </span>
              </div>
              <div className="tor-tree">{tree.map((node) => renderNode(node, 0))}</div>
            </div>
            {chosen.size === 0 && <p className="hint warning">{t("torrentNoFile")}</p>}

            <div className="field">
              <span>{t("saveIn")}</span>
              <div className="input-row">
                <input value={folder} spellCheck={false} onChange={(e) => setDirectory(e.target.value)} />
                <button type="button" className="button" onClick={browse}>
                  <Icon name="folder" size={16} />
                  {t("browse")}
                </button>
              </div>
            </div>
            <p className="hint faint">{t("torrentRights")}</p>
            <button type="submit" hidden />
          </form>
        )
      )}
    </Dialog>
  );
}
