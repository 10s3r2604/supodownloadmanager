// The files and peers of a torrent download, for the details panel.

import { useEffect, useState } from "react";

import { type Download, type TorrentFile, api } from "../api";
import { useI18n } from "../context";
import { formatBytes, formatSpeed } from "../format";
import { Icon, categoryIcon } from "../icons";
import { categoryOf } from "../model";

const cache = new Map<number, TorrentFile[]>();

/** The files of a torrent download; they never change, so ask once. */
export function useTorrentFiles(d: Download): TorrentFile[] {
  const [files, setFiles] = useState<TorrentFile[]>(() => cache.get(d.id) ?? []);
  useEffect(() => {
    if (!d.torrent) return;
    const known = cache.get(d.id);
    if (known) {
      setFiles(known);
      return;
    }
    let alive = true;
    api
      .torrentFiles(d.id)
      .then((list) => {
        cache.set(d.id, list);
        if (alive) setFiles(list);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [d.id, d.torrent]);
  return files;
}

export function TorrentPanel({ d }: { d: Download }) {
  const { t, language } = useI18n();
  const files = useTorrentFiles(d);
  const running = d.state === "downloading";
  // One segment per file, unless there are too many files to show.
  const perFile = d.segments.length === files.length;
  const done = (i: number, size: number) =>
    d.state === "completed" ? size : perFile ? Math.min(size, d.segments[i].downloaded) : null;

  return (
    <div className="torrent-panel">
      <div className="torrent-stats mono">
        {running && (
          <span>
            <Icon name="peers" size={14} /> {t("peersTitle", { n: d.connections })}
          </span>
        )}
        {(running || d.seeding) && (
          <span>
            <Icon name="upload" size={14} /> {formatSpeed(d.uploadSpeed, language)}
          </span>
        )}
        <span title={t("uploadedLabel")}>
          {t("uploadedLabel")}: {formatBytes(d.uploaded, language)}
        </span>
      </div>
      <h3>
        {t("filesTitle")} · {files.length}
      </h3>
      <div className="torrent-files">
        {files.map((file, i) => {
          const have = done(i, file.size);
          const name = file.path.split("/").pop() ?? file.path;
          return (
            <div key={file.index} className="torrent-file" title={file.path}>
              <Icon name={categoryIcon[categoryOf(name)]} size={14} />
              <span className="name">{file.path}</span>
              <span className="bar">
                {have != null && <b style={{ width: `${(have / Math.max(1, file.size)) * 100}%` }} />}
              </span>
              <span className="size mono">{formatBytes(file.size, language)}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
