// Picks the quality of a video on a web page ("Download this video").

import { useCallback, useEffect, useState } from "react";

import { type MediaInfo, type MediaOption, type MediaSource, type Settings, type ToolProgress, type ToolStatus, api } from "../api";
import { errorText, useI18n } from "../context";
import { formatBytes, formatDuration } from "../format";
import { Icon } from "../icons";
import { hostOf } from "../model";
import { Dialog } from "./Dialog";

interface Props {
  source: MediaSource;
  settings: Settings;
  onClose: () => void;
  onError: (message: string) => void;
  /** Hands a direct video link to the normal download dialog. */
  onDirect: (url: string) => void;
}

type Stage =
  | { kind: "tools"; progress: ToolProgress | null }
  | { kind: "probing" }
  | { kind: "ready" }
  | { kind: "ffmpeg"; progress: ToolProgress | null }
  | { kind: "failed"; error: string };

export function MediaDialog({ source, settings, onClose, onError, onDirect }: Props) {
  const { t, language } = useI18n();
  const [stage, setStage] = useState<Stage>({ kind: "probing" });
  const [tools, setTools] = useState<ToolStatus | null>(null);
  const [info, setInfo] = useState<MediaInfo | null>(null);
  const [choice, setChoice] = useState<string | null>(null);
  const [title, setTitle] = useState("");
  const [directory, setDirectory] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // What yt-dlp is doing, so a slow site is not a mystery.
  const [ytdlpStage, setYtdlpStage] = useState<string | null>(null);
  const probing = stage.kind === "probing";
  useEffect(() => {
    if (!probing) {
      setYtdlpStage(null);
      return;
    }
    const timer = setInterval(() => {
      api.mediaProbeStage(source.pageUrl).then(setYtdlpStage).catch(() => {});
    }, 700);
    return () => clearInterval(timer);
  }, [probing, source.pageUrl]);

  // Tool downloads report their progress with events.
  useEffect(() => {
    const stop = api.onToolProgress((progress) =>
      setStage((current) =>
        current.kind === "tools" || current.kind === "ffmpeg" ? { ...current, progress } : current,
      ),
    );
    return () => {
      stop.then((unlisten) => unlisten());
    };
  }, []);

  const load = useCallback(async () => {
    try {
      let status = await api.mediaTools();
      setTools(status);
      if (!status.ytdlp) {
        setStage({ kind: "tools", progress: null });
        status = await api.mediaPrepare(false);
      }
      setTools(status);
      setStage({ kind: "probing" });
      const found = await api.mediaProbe(source);
      setInfo(found);
      setTitle(found.title);
      setChoice(found.default ?? found.options[0]?.id ?? null);
      setStage({ kind: "ready" });
    } catch (error) {
      setStage({ kind: "failed", error: errorText(error) });
    }
  }, [source]);

  useEffect(() => {
    load();
  }, [load]);

  const option = info?.options.find((o) => o.id === choice) ?? null;
  const ffmpegMissing = Boolean(option?.needsFfmpeg && tools && !tools.ffmpeg);
  const blocked = ffmpegMissing && !tools?.ffmpegInstallable;
  const folder = directory ?? (option?.kind === "audio" ? info?.audioDir : info?.videoDir) ?? settings.downloadDir;

  async function browse() {
    const picked = await api.pickFolder();
    if (picked) setDirectory(picked);
  }

  async function submit(start: boolean) {
    if (!option || busy || blocked) return;
    setBusy(true);
    try {
      if (ffmpegMissing) {
        setStage({ kind: "ffmpeg", progress: null });
        setTools(await api.mediaPrepare(true));
      }
      await api.addMedia({
        source,
        option,
        title: title.trim() || info?.title || "video",
        directory: directory ?? undefined,
        start,
      });
      onClose();
    } catch (error) {
      onError(errorText(error));
      setStage({ kind: "ready" });
      setBusy(false);
    }
  }

  const ready = stage.kind === "ready";
  const direct = source.mediaUrl && /^https?:/i.test(source.mediaUrl) ? source.mediaUrl : null;

  return (
    <Dialog
      title={t("mediaTitle")}
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            {t("cancel")}
          </button>
          {ready && (
            <>
              <button type="button" className="button" disabled={!option || busy || blocked} onClick={() => submit(false)}>
                {t("downloadLater")}
              </button>
              <button type="button" className="button primary" disabled={!option || busy || blocked} onClick={() => submit(true)}>
                <Icon name="download" size={16} />
                {t("downloadNow")}
              </button>
            </>
          )}
        </>
      }
    >
      {stage.kind === "tools" || stage.kind === "ffmpeg" ? (
        <ToolDownload
          text={
            stage.kind === "tools"
              ? t("mediaPreparing", { size: tools?.ytdlpSizeMb ?? 20 })
              : t("mediaFfmpeg")
          }
          progress={stage.progress}
        />
      ) : stage.kind === "probing" ? (
        <div className="media-wait">
          <span className="spinner" />
          <div>
            <strong>{t("mediaProbing")}</strong>
            <div className="muted">{source.title || hostOf(source.pageUrl)}</div>
            {ytdlpStage && <div className="media-stage">{ytdlpStage}</div>}
          </div>
        </div>
      ) : stage.kind === "failed" ? (
        <div className="media-failed">
          <p className="warning">
            <Icon name="alert" size={16} /> {t("mediaFailed", { error: stage.error })}
          </p>
          <div className="media-failed-actions">
            <button type="button" className="button" onClick={load}>
              <Icon name="retry" size={15} /> {t("mediaRetry")}
            </button>
            {direct && (
              <button type="button" className="button" onClick={() => onDirect(direct)}>
                <Icon name="download" size={15} /> {t("mediaDirect")}
              </button>
            )}
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
              <div className="media-thumb">
                {info.thumbnail ? <img src={info.thumbnail} alt="" /> : <Icon name="video" size={28} />}
                {info.duration != null && <span>{formatDuration(info.duration)}</span>}
              </div>
              <div className="media-about">
                <div className="media-title" title={info.title}>
                  {info.title}
                </div>
                <div className="muted">
                  {[siteName(info.site, info.url), info.duration != null && formatDuration(info.duration)]
                    .filter(Boolean)
                    .join(" · ")}
                </div>
              </div>
            </div>

            <div className="media-options" role="radiogroup">
              {(["video", "audio"] as const).map((kind) => {
                const list = info.options.filter((o) => o.kind === kind);
                if (list.length === 0) return null;
                return (
                  <div key={kind} className="media-group">
                    <div className="media-group-title">{kind === "video" ? t("mediaVideo") : t("mediaAudio")}</div>
                    {list.map((o) => (
                      <OptionRow
                        key={o.id}
                        option={o}
                        selected={o.id === choice}
                        unavailable={Boolean(o.needsFfmpeg && tools && !tools.ffmpeg && !tools.ffmpegInstallable)}
                        onSelect={() => setChoice(o.id)}
                        language={language}
                      />
                    ))}
                  </div>
                );
              })}
            </div>

            <label className="field">
              <span>{t("fileName")}</span>
              <div className="input-row">
                <input autoFocus value={title} spellCheck={false} onChange={(e) => setTitle(e.target.value)} />
                <span className="media-ext">.{option?.ext ?? "mp4"}</span>
              </div>
            </label>
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

            {ffmpegMissing && (
              <p className={`hint ${blocked ? "warning" : "muted"}`}>
                <Icon name={blocked ? "alert" : "download"} size={14} />{" "}
                {blocked ? t("mediaFfmpegMissing") : t("mediaFfmpegDownload", { size: tools?.ffmpegSizeMb ?? 66 })}
              </p>
            )}
            <p className="hint faint">{t("mediaRights")}</p>
            <button type="submit" hidden />
          </form>
        )
      )}
    </Dialog>
  );
}

function siteName(site: string, url: string) {
  if (!site || site === "Generic" || site === "HTML5MediaEmbed") return hostOf(url);
  return site;
}

function OptionRow(props: {
  option: MediaOption;
  selected: boolean;
  unavailable: boolean;
  onSelect: () => void;
  language: Parameters<typeof formatBytes>[1];
}) {
  const { t } = useI18n();
  const o = props.option;
  const label =
    o.kind === "audio"
      ? o.ext.toUpperCase()
      : o.height
        ? `${o.height}p${o.fps && o.fps > 30 ? o.fps : ""}`
        : t("mediaBest");
  const badge = o.height && o.height >= 2160 ? "4K" : o.height && o.height >= 1440 ? "2K" : o.height && o.height >= 720 ? "HD" : null;
  const detail = [o.vcodec, o.acodec, o.kind === "video" ? o.ext.toUpperCase() : null].filter(Boolean).join(" · ");
  return (
    <label className={`media-option${props.selected ? " on" : ""}${props.unavailable ? " off" : ""}`}>
      <input type="radio" name="media-option" checked={props.selected} disabled={props.unavailable} onChange={props.onSelect} />
      <span className="media-label">
        {label}
        {badge && <em>{badge}</em>}
      </span>
      <span className="media-detail">{detail}</span>
      <span className="media-size">{o.size == null ? "—" : `~${formatBytes(o.size, props.language)}`}</span>
    </label>
  );
}

function ToolDownload({ text, progress }: { text: string; progress: ToolProgress | null }) {
  const { language } = useI18n();
  const fraction = progress?.total ? Math.min(1, progress.downloaded / progress.total) : null;
  return (
    <div className="media-wait column">
      <div className="media-wait-row">
        <span className="spinner" />
        <strong>{text}</strong>
      </div>
      <div className="media-bar">
        <i style={{ width: `${(fraction ?? 0) * 100}%` }} />
      </div>
      <div className="muted">
        {progress
          ? `${formatBytes(progress.downloaded, language)}${progress.total ? ` / ${formatBytes(progress.total, language)}` : ""}`
          : "…"}
      </div>
    </div>
  );
}
