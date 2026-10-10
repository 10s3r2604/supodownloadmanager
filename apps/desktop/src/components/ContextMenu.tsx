import { useEffect, useLayoutEffect, useRef, useState } from "react";

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
  const menu = useRef<HTMLDivElement>(null);
  const opener = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  const [position, setPosition] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const box = menu.current!;
    const rect = box.getBoundingClientRect();
    setPosition({
      left: Math.max(8, Math.min(x, window.innerWidth - rect.width - 8)),
      top: Math.max(8, Math.min(y, window.innerHeight - rect.height - 8)),
    });
    box.querySelector<HTMLButtonElement>("button")?.focus();
    return () => {
      if (opener.current?.isConnected) opener.current.focus({ preventScroll: true });
    };
  }, [x, y]);

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
      ref={menu}
      className="context-menu"
      style={position}
      role="menu"
      aria-label={`${t("more")}: ${d.fileName}`}
      onKeyDown={(event) => {
        const items = [...menu.current!.querySelectorAll<HTMLButtonElement>("button")];
        const current = items.indexOf(document.activeElement as HTMLButtonElement);
        if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
          event.preventDefault();
          const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
            : (current + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
          items[next]?.focus();
        } else if (event.key === "Escape" || event.key === "Tab") {
          event.preventDefault();
          onClose();
        }
      }}
    >
      {item("file", t("pwDetails"), () => actions.details(d))}
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
