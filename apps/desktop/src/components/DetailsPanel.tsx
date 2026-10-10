import { useState } from "react";

import type { Download } from "../api";
import { copyText, useI18n } from "../context";
import { formatBytes, percent } from "../format";
import { Icon } from "../icons";
import { statusText } from "./DownloadTable";

interface Props {
  download: Download;
  onOpen: () => void;
  onShowInFolder: () => void;
}

export function DetailsPanel({ download: d, onOpen, onShowInFolder }: Props) {
  const { t, language } = useI18n();
  const [copied, setCopied] = useState(false);
  const value = percent(d.downloaded, d.size);

  return (
    <section className="details" aria-label={d.fileName}>
      <div className="details-head">
        <div className="details-title">
          <strong title={d.fileName}>{d.fileName}</strong>
          <span className={`status status-${d.state}`}>{statusText(d, t)}</span>
        </div>
        <div className="details-actions">
          {d.state === "completed" && (
            <button type="button" className="button small" onClick={onOpen}>
              <Icon name="open" size={14} /> {t("open")}
            </button>
          )}
          <button type="button" className="button small" onClick={onShowInFolder}>
            <Icon name="folder" size={14} /> {t("showInFolder")}
          </button>
        </div>
      </div>

      <SegmentMap download={d} />

      <dl className="details-grid">
        <dt>{t("link")}</dt>
        <dd className="with-action">
          <span className="truncate" title={d.url}>
            {d.url}
          </span>
          <button
            type="button"
            className="icon-button"
            aria-label={t("copy")}
            title={copied ? t("copied") : t("copy")}
            onClick={async () => {
              await copyText(d.url);
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            }}
          >
            <Icon name={copied ? "check" : "copy"} size={14} />
          </button>
        </dd>
        <dt>{t("savedTo")}</dt>
        <dd className="truncate" title={d.path}>
          {d.path}
        </dd>
        <dt>{t("colSize")}</dt>
        <dd>
          {formatBytes(d.downloaded, language)}
          {d.size != null && ` / ${formatBytes(d.size, language)}`}
          {value != null && ` (${value.toFixed(1)}%)`}
        </dd>
        <dt>{t("resumable")}</dt>
        <dd>{d.resumable ? t("resumableYes") : t("resumableNo")}</dd>
        {d.state === "downloading" && (
          <>
            <dt>{t("connections")}</dt>
            <dd>{d.connections}</dd>
          </>
        )}
        {d.error && (
          <>
            <dt>{t("error")}</dt>
            <dd className="error-text">{d.error}</dd>
          </>
        )}
      </dl>
    </section>
  );
}

/** Shows which parts of the file are done, one bar per connection. */
function SegmentMap({ download: d }: { download: Download }) {
  const { t } = useI18n();
  if (!d.size) return null;
  const segments =
    d.segments.length > 0 ? d.segments : [{ start: 0, end: d.size, downloaded: Math.min(d.downloaded, d.size) }];
  return (
    <div className="segment-map" role="img" aria-label={t("segments")}>
      {segments.map((segment) => {
        const length = Math.max(1, segment.end - segment.start);
        return (
          <div
            key={segment.start}
            className="segment"
            style={{
              left: `${(segment.start / d.size!) * 100}%`,
              width: `${(length / d.size!) * 100}%`,
            }}
          >
            <div
              className={`segment-fill ${d.state}`}
              style={{ width: `${Math.min(100, (segment.downloaded / length) * 100)}%` }}
            />
          </div>
        );
      })}
    </div>
  );
}
