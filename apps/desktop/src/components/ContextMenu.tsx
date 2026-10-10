import { useEffect } from "react";

import type { Download } from "../api";
import { useI18n } from "../context";
import { Icon } from "../icons";
import type { RowActions } from "../views";

interface Props {
  x: number;
  y: number;
  download: Download;
  selected: number[];
  actions: RowActions;
  onClose: () => void;
}

export function ContextMenu({ x, y, download: d, selected, actions, onClose }: Props) {
  const { t } = useI18n();
  const ids = selected.includes(d.id) ? selected : [d.id];
  const canPause = d.state === "downloading" || d.state === "queued";
  const canResume = d.state === "paused" || d.state === "failed";

  useEffect(() => {
    const close = () => onClose();
    const timer = setTimeout(() => window.addEventListener("click", close), 0);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("click", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
    };
  }, [onClose]);

  const item = (icon: Parameters<typeof Icon>[0]["name"], label: string, run: () => void, danger = false) => (
    <button type="button" role="menuitem" className={danger ? "danger" : ""} onClick={run}>
      <Icon name={icon} size={15} /> {label}
    </button>
  );

  return (
    <div
      className="context-menu"
      style={{ left: Math.min(x, window.innerWidth - 230), top: Math.min(y, window.innerHeight - 260) }}
      role="menu"
    >
      {d.state === "completed" && item("open", t("open"), () => actions.open(d))}
      {d.state === "downloading" && item("window", t("viewProgress"), () => actions.openProgress(d))}
      {item("folder", t("showInFolder"), () => actions.showInFolder(d))}
      {canPause && item("pause", t("pause"), () => actions.pause(ids))}
      {canResume && item("play", t("resume"), () => actions.resume(ids))}
      {d.seeding && item("pause", t("stopSeeding"), () => actions.pause([d.id]))}
      {item("link", t("copyLink"), () => actions.copyLink(d))}
      <hr />
      {item("trash", t("delete"), () => actions.remove(ids), true)}
    </div>
  );
}
