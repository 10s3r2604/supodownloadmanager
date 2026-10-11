import { useEffect, useState } from "react";

import { type AddPrompt, MAIN_QUEUE, type ProbeResult, type Queue, type Settings, type TorrentSource, api } from "../api";
import { errorText, useI18n } from "../context";
import { formatBytes } from "../format";
import { Icon, categoryIcon } from "../icons";
import { Dialog } from "./Dialog";
import { queueName } from "./SchedulerDialog";

interface Props {
  prompt?: AddPrompt;
  initialUrl?: string;
  settings: Settings;
  queues: Queue[];
  onClose: () => void;
  onError: (message: string) => void;
  /** Switches to the video quality picker for a page. */
  onMedia: (pageUrl: string) => void;
  /** Switches to the torrent dialog. */
  onTorrent: (source: TorrentSource) => void;
  /** Switches to the batch download dialog. */
  onBatch: () => void;
}

const isLink = (text: string) => /^https?:\/\/\S+$/i.test(text.trim());
const isMagnet = (text: string) => /^magnet:\?\S+$/i.test(text.trim());
const isTorrentFile = (name: string) => /\.torrent$/i.test(name.split(/[?#]/)[0] ?? "");

/** Sites whose pages hold videos rather than files. */
const VIDEO_SITES =
  /(^|\.)(youtube\.com|youtu\.be|facebook\.com|fb\.watch|x\.com|twitter\.com|tiktok\.com|instagram\.com|vimeo\.com|dailymotion\.com|bilibili\.com|twitch\.tv|soundcloud\.com|reddit\.com|threads\.net|zingmp3\.vn|nhaccuatui\.com)$/i;

function isVideoPage(url: string) {
  try {
    return VIDEO_SITES.test(new URL(url.trim()).hostname);
  } catch {
    return false;
  }
}

export function AddDialog({ prompt, initialUrl, settings, queues, onClose, onError, onMedia, onTorrent, onBatch }: Props) {
  const { t, language } = useI18n();
  const headers = prompt?.headers ?? [];
  const [url, setUrl] = useState(prompt?.url ?? initialUrl ?? "");
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [probing, setProbing] = useState(false);
  const [probeError, setProbeError] = useState<string | null>(null);
  const [fileName, setFileName] = useState(prompt?.fileName ?? "");
  const [nameEdited, setNameEdited] = useState(false);
  const [directory, setDirectory] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [queue, setQueue] = useState(MAIN_QUEUE);
  const valid = isLink(url);

  // Magnet links have no server to ask; they go to the torrent dialog.
  useEffect(() => {
    if (isMagnet(url)) onTorrent({ link: url.trim() });
  }, [url, onTorrent]);

  useEffect(() => {
    setProbe(null);
    setProbeError(null);
    if (!isLink(url)) {
      setProbing(false);
      return;
    }
    let cancelled = false;
    setProbing(true);
    const timer = setTimeout(async () => {
      try {
        const result = await api.probe(url.trim(), headers);
        if (cancelled) return;
        setProbe(result);
        if (!nameEdited) setFileName(result.fileName);
      } catch (error) {
        if (!cancelled) setProbeError(errorText(error));
      } finally {
        if (!cancelled) setProbing(false);
      }
    }, 350);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
    // Only the link decides what to ask the server.
  }, [url]);

  const shownDirectory = directory ?? probe?.directory ?? settings.downloadDir;

  async function browse() {
    const folder = await api.pickFolder();
    if (folder) setDirectory(folder);
  }

  async function openTorrentFile() {
    const path = await api.pickTorrentFile();
    if (path) onTorrent({ path });
  }

  const torrentLink =
    valid &&
    (isTorrentFile(url) ||
      probe?.contentType?.startsWith("application/x-bittorrent") ||
      (probe != null && isTorrentFile(probe.fileName)));

  async function submit(start: boolean) {
    if (!valid || busy) return;
    setBusy(true);
    try {
      await api.add({
        url: url.trim(),
        headers,
        fileName: fileName.trim() || undefined,
        directory: directory ?? undefined,
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
      title={t("addTitle")}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            {t("cancel")}
          </button>
          <button type="button" className="button" disabled={!valid || busy} onClick={() => submit(false)}>
            {t("downloadLater")}
          </button>
          <button type="button" className="button primary" disabled={!valid || busy} onClick={() => submit(true)}>
            <Icon name="download" size={16} />
            {t("downloadNow")}
          </button>
        </>
      }
    >
      <form
        className="form"
        onSubmit={(event) => {
          event.preventDefault();
          submit(true);
        }}
      >
        <label className="field">
          <span>{t("urlLabel")}</span>
          <input
            autoFocus
            value={url}
            placeholder="https://… / magnet:?…"
            spellCheck={false}
            onChange={(event) => setUrl(event.target.value)}
          />
        </label>
        <div className="add-extra">
          {!prompt && (
            <button type="button" className="link-button" onClick={onBatch}>
              <Icon name="layers" size={14} />
              {t("batch")}
            </button>
          )}
          <button type="button" className="link-button" onClick={openTorrentFile}>
            <Icon name="magnet" size={14} />
            {t("openTorrentFile")}
          </button>
        </div>
        {url.trim() && !valid && !isMagnet(url) && <p className="hint warning">{t("invalidUrl")}</p>}
        {torrentLink && (
          <div className="video-banner">
            <Icon name="magnet" size={18} />
            <span>{t("addTorrentHint")}</span>
            <button type="button" className="button primary" onClick={() => onTorrent({ link: url.trim(), headers })}>
              {t("addTorrentButton")}
            </button>
          </div>
        )}
        {valid && (isVideoPage(url) || probe?.contentType?.startsWith("text/html")) && (
          <div className="video-banner">
            <Icon name="video" size={18} />
            <span>{isVideoPage(url) ? t("addVideoHint") : t("addPageHint")}</span>
            <button type="button" className="button primary" onClick={() => onMedia(url.trim())}>
              {isVideoPage(url) ? t("addVideoButton") : t("addPageButton")}
            </button>
          </div>
        )}

        <div className="probe">
          {probing && <span className="muted">{t("checking")}</span>}
          {probeError && (
            <span className="warning">
              <Icon name="alert" size={14} /> {t("probeFailed", { error: probeError })}
            </span>
          )}
          {probe && !probing && (
            <>
              <span className="chip">
                <Icon name={categoryIcon[probe.category]} size={14} />
                {t(`cat.${probe.category}`)}
              </span>
              <span className="chip">{probe.size == null ? t("unknownSize") : formatBytes(probe.size, language)}</span>
              <span className={`chip ${probe.resumable ? "good" : "warn"}`}>
                {probe.resumable ? t("resumableYes") : t("resumableNo")}
              </span>
            </>
          )}
          {headers.length > 0 && (
            <span className="chip">
              <Icon name="globe" size={14} />
              {t("fromBrowser")}
            </span>
          )}
        </div>

        <label className="field">
          <span>{t("fileName")}</span>
          <input
            value={fileName}
            spellCheck={false}
            onChange={(event) => {
              setFileName(event.target.value);
              setNameEdited(true);
            }}
          />
        </label>

        <div className="field">
          <span>{t("saveIn")}</span>
          <div className="input-row">
            <input value={shownDirectory} spellCheck={false} onChange={(event) => setDirectory(event.target.value)} />
            <button type="button" className="button" onClick={browse}>
              <Icon name="folder" size={16} />
              {t("browse")}
            </button>
          </div>
        </div>
        {queues.length > 1 && (
          <label className="field narrow">
            <span>{t("queue")}</span>
            <select value={queue} onChange={(event) => setQueue(event.target.value)}>
              {queues.map((q) => (
                <option key={q.id} value={q.id}>
                  {queueName(q, t)}
                </option>
              ))}
            </select>
          </label>
        )}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}
