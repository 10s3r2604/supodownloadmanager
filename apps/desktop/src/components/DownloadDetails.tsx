import type { Download } from "../api";
import { useI18n } from "../context";
import { formatBytes, formatHuman, formatSpeed } from "../format";
import { Icon } from "../icons";
import { blocks, hostOf, type History } from "../model";
import type { RowActions } from "../views";
import { AreaChart, niceMax, SegmentBlocks } from "./Charts";
import { Dialog } from "./Dialog";
import { TorrentPanel } from "./TorrentPanel";
import "./download-details.css";

export function DownloadDetails({
  d,
  history,
  actions,
  onClose,
}: {
  d: Download | undefined;
  history: History;
  actions: RowActions;
  onClose: () => void;
}) {
  const { t, language } = useI18n();
  const running = d?.state === "downloading";
  const values = d ? (history.perDownload.get(d.id) ?? []) : [];
  const speeds = [
    ...(d ? (history.segments.get(d.id) ?? new Map<number, number>()) : []),
  ].sort((a, b) => a[0] - b[0]);
  const fastest = Math.max(1, ...speeds.map(([, v]) => v));

  return (
    <Dialog
      title={t("pwDetails")}
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            {t("close")}
          </button>
          {d?.seeding && (
            <button
              type="button"
              className="button"
              disabled={d.stopping}
              onClick={() => actions.pause([d.id])}
            >
              {t("stopSeeding")}
            </button>
          )}
          {d && (
            <button
              type="button"
              className="button primary"
              disabled={d.stopping}
              onClick={() => actions.primary(d)}
            >
              {running || d.state === "queued"
                ? t("actPause")
                : d.state === "completed"
                  ? t("actOpen")
                  : d.state === "failed"
                    ? t("actRetry")
                    : t("actResume")}
            </button>
          )}
        </>
      }
    >
      {!d ? (
        <p>{t("pwGone")}</p>
      ) : (
        <div className="download-details">
          <h3>{d.fileName}</h3>
          <dl className="download-kv">
            <dt>{t("colStatus")}</dt>
            <dd>
              {d.stopping
                ? t("stopping")
                : d.seeding
                  ? t("seeding")
                  : d.checking
                    ? t("checkingData")
                    : d.processing
                      ? t("processing")
                      : t(`state.${d.state}`)}
            </dd>
            <dt>{t("downloadedLabel")}</dt>
            <dd>
              {formatBytes(d.downloaded, language)} /{" "}
              {d.size == null
                ? t("unknownSize")
                : formatBytes(d.size, language)}
            </dd>
            {running && (
              <>
                <dt>{t("colSpeed")}</dt>
                <dd>{formatSpeed(d.speed, language)}</dd>
              </>
            )}
            {running && d.eta != null && (
              <>
                <dt>{t("timeLeft")}</dt>
                <dd>{formatHuman(d.eta, language)}</dd>
              </>
            )}
            <dt>{t("savedTo")}</dt>
            <dd>{d.path}</dd>
            <dt>{t("link")}</dt>
            <dd>{d.url}</dd>
            {!d.torrent && (
              <>
                <dt>{t("server")}</dt>
                <dd>{hostOf(d.url)}</dd>
                <dt>{t("resumable")}</dt>
                <dd>{t(d.resumable ? "resumableYes" : "resumableNo")}</dd>
              </>
            )}
            {d.error && (
              <>
                <dt>{t("error")}</dt>
                <dd className="mn-error">{d.error}</dd>
              </>
            )}
          </dl>
          <div className="download-tools">
            <button
              type="button"
              className="button"
              onClick={() => actions.showInFolder(d)}
            >
              <Icon name="folder" size={16} />
              {t("showInFolder")}
            </button>
            <button
              type="button"
              className="button"
              onClick={() => actions.copyLink(d)}
            >
              <Icon name="link" size={16} />
              {t("copyLink")}
            </button>
            {(running || d.state === "queued") && (
              <button
                type="button"
                className="button"
                onClick={() => actions.openProgress(d)}
              >
                <Icon name="window" size={16} />
                {t("viewProgress")}
              </button>
            )}
            <button
              type="button"
              className="button"
              onClick={() => actions.remove([d.id])}
            >
              <Icon name="trash" size={16} />
              {t("delete")}
            </button>
          </div>
          {running && (
            <div
              className="download-chart"
              role="img"
              aria-label={`${t("colSpeed")}: ${formatSpeed(d.speed, language)}`}
            >
              <AreaChart
                values={values}
                max={niceMax(Math.max(0, ...values))}
                slots={30}
                id={`download-speed-${d.id}`}
              />
              <span>
                {t("colSpeed")} · {formatSpeed(d.speed, language)}
              </span>
            </div>
          )}
          {d.torrent ? (
            <TorrentPanel d={d} />
          ) : (
            <>
              <h4>{t("colSegments")}</h4>
              {d.size && d.segments.length > 0 ? (
                <div className={`segment-map ${d.state}`}>
                  {d.segments.map((s) => (
                    <i
                      key={s.start}
                      style={{
                        left: `${(s.start / d.size!) * 100}%`,
                        width: `${((s.end - s.start) / d.size!) * 100}%`,
                      }}
                    >
                      <b
                        style={{
                          width: `${Math.min(100, (s.downloaded / Math.max(1, s.end - s.start)) * 100)}%`,
                        }}
                      />
                    </i>
                  ))}
                </div>
              ) : (
                <SegmentBlocks
                  fills={blocks(d, 8)}
                  className={`big ${d.state}`}
                />
              )}
              {running && (
                <>
                  <h4>{t("connectionsTitle", { n: d.connections })}</h4>
                  {speeds.length === 0 && (
                    <p className="muted">{t("connecting")}</p>
                  )}
                  {speeds.map(([start, speed], i) => (
                    <div key={start} className="download-connection">
                      <span>{t("segmentN", { n: i + 1 })}</span>
                      <span className="bar">
                        <b style={{ width: `${(speed / fastest) * 100}%` }} />
                      </span>
                      <span>{formatSpeed(speed, language)}</span>
                    </div>
                  ))}
                </>
              )}
            </>
          )}
        </div>
      )}
    </Dialog>
  );
}
