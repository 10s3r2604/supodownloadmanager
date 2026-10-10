import { useEffect, useState } from "react";

import type { Download } from "../api";
import { useI18n } from "../context";
import { formatBytes, formatDate, formatDuration, percent } from "../format";
import { Icon, categoryIcon } from "../icons";

export interface RowActions {
  select: (id: number, mode: "single" | "toggle" | "range") => void;
  open: (download: Download) => void;
  showInFolder: (download: Download) => void;
  pause: (ids: number[]) => void;
  resume: (ids: number[]) => void;
  copyLink: (download: Download) => void;
  remove: (ids: number[]) => void;
}

interface Props {
  downloads: Download[];
  selected: number[];
  empty: { title: string; body?: string };
  actions: RowActions;
}

export function statusText(download: Download, t: ReturnType<typeof useI18n>["t"]) {
  if (download.stopping) return t("stopping");
  return t(`state.${download.state}`);
}

export function DownloadTable({ downloads, selected, empty, actions }: Props) {
  const { t, language } = useI18n();
  const [menu, setMenu] = useState<{ x: number; y: number; download: Download } | null>(null);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("click", close);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
    };
  }, [menu]);

  if (downloads.length === 0) {
    return (
      <div className="empty">
        <Icon name="download" size={40} />
        <h3>{empty.title}</h3>
        {empty.body && <p>{empty.body}</p>}
      </div>
    );
  }

  return (
    <div className="table-wrap">
      <table className="downloads">
        <thead>
          <tr>
            <th className="col-name">{t("colName")}</th>
            <th className="col-size">{t("colSize")}</th>
            <th className="col-progress">{t("colProgress")}</th>
            <th className="col-speed">{t("colSpeed")}</th>
            <th className="col-eta">{t("colEta")}</th>
            <th className="col-status">{t("colStatus")}</th>
            <th className="col-added">{t("colAdded")}</th>
          </tr>
        </thead>
        <tbody>
          {downloads.map((d) => {
            const value = percent(d.downloaded, d.size);
            const isSelected = selected.includes(d.id);
            return (
              <tr
                key={d.id}
                className={`${isSelected ? "selected " : ""}state-${d.state}`}
                aria-selected={isSelected}
                onMouseDown={(event) => {
                  if (event.button !== 0) return;
                  actions.select(d.id, event.shiftKey ? "range" : event.ctrlKey || event.metaKey ? "toggle" : "single");
                }}
                onDoubleClick={() => (d.state === "completed" ? actions.open(d) : actions.showInFolder(d))}
                onContextMenu={(event) => {
                  event.preventDefault();
                  if (!isSelected) actions.select(d.id, "single");
                  setMenu({ x: event.clientX, y: event.clientY, download: d });
                }}
              >
                <td className="col-name">
                  <div className="cell-name">
                    <span className={`file-icon cat-${d.category}`}>
                      <Icon name={categoryIcon[d.category]} size={16} />
                    </span>
                    <span className="file-name" title={d.fileName}>
                      {d.fileName}
                    </span>
                  </div>
                </td>
                <td className="col-size">{formatBytes(d.size, language)}</td>
                <td className="col-progress">
                  <div className="cell-progress">
                    <div className="progress" title={value == null ? undefined : `${value.toFixed(1)}%`}>
                      <div
                        className={`progress-fill ${d.state}`}
                        style={{ width: `${value ?? (d.state === "downloading" ? 100 : 0)}%` }}
                      />
                    </div>
                    <span className="progress-text">
                      {value == null ? formatBytes(d.downloaded, language) : `${Math.floor(value)}%`}
                    </span>
                  </div>
                </td>
                <td className="col-speed">
                  {d.state === "downloading" && d.speed > 0 ? `${formatBytes(d.speed, language)}/s` : ""}
                </td>
                <td className="col-eta">{d.state === "downloading" ? formatDuration(d.eta) : ""}</td>
                <td className="col-status" title={d.error ?? undefined}>
                  <span className={`status status-${d.state}`}>{statusText(d, t)}</span>
                </td>
                <td className="col-added">{formatDate(d.addedAt, language)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          download={menu.download}
          selected={selected}
          actions={actions}
        />
      )}
    </div>
  );
}

function ContextMenu(props: { x: number; y: number; download: Download; selected: number[]; actions: RowActions }) {
  const { t } = useI18n();
  const { download: d, actions } = props;
  const ids = props.selected.includes(d.id) ? props.selected : [d.id];
  const canPause = d.state === "downloading" || d.state === "queued";
  const canResume = d.state === "paused" || d.state === "failed";
  const style = {
    left: Math.min(props.x, window.innerWidth - 220),
    top: Math.min(props.y, window.innerHeight - 220),
  };
  return (
    <div className="context-menu" style={style} role="menu">
      {d.state === "completed" && (
        <button type="button" role="menuitem" onClick={() => actions.open(d)}>
          <Icon name="open" size={15} /> {t("open")}
        </button>
      )}
      <button type="button" role="menuitem" onClick={() => actions.showInFolder(d)}>
        <Icon name="folder" size={15} /> {t("showInFolder")}
      </button>
      {canPause && (
        <button type="button" role="menuitem" onClick={() => actions.pause(ids)}>
          <Icon name="pause" size={15} /> {t("pause")}
        </button>
      )}
      {canResume && (
        <button type="button" role="menuitem" onClick={() => actions.resume(ids)}>
          <Icon name="play" size={15} /> {t("resume")}
        </button>
      )}
      <button type="button" role="menuitem" onClick={() => actions.copyLink(d)}>
        <Icon name="link" size={15} /> {t("copyLink")}
      </button>
      <hr />
      <button type="button" role="menuitem" className="danger" onClick={() => actions.remove(ids)}>
        <Icon name="trash" size={15} /> {t("delete")}
      </button>
    </div>
  );
}
