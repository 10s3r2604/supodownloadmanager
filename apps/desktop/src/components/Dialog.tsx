import { type ReactNode, useLayoutEffect, useRef } from "react";

import { useI18n } from "../context";
import { Icon } from "../icons";

interface Props {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
  standalone?: boolean;
}

export function Dialog({ title, onClose, children, footer, wide, standalone }: Props) {
  const { t } = useI18n();
  const dialog = useRef<HTMLDivElement>(null);
  const initialFocus = useRef<HTMLElement | null>(null);
  const close = useRef(onClose);
  close.current = onClose;
  const opener = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);

  useLayoutEffect(() => {
    const box = dialog.current!;
    const focusable = () => [...box.querySelectorAll<HTMLElement>(
      'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], summary, [tabindex="0"]',
    )].filter((el) => el.getClientRects().length > 0 && !el.hidden);
    // Remember child autofocus across StrictMode's effect cleanup/setup cycle.
    initialFocus.current ??= box.contains(document.activeElement)
      ? document.activeElement as HTMLElement : focusable()[0] ?? box;
    if (!box.contains(document.activeElement)) initialFocus.current.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close.current();
      } else if (event.key === "Tab") {
        const items = focusable();
        const first = items[0] ?? box;
        const last = items.at(-1) ?? box;
        if (items.length === 0 || !box.contains(document.activeElement) ||
          (event.shiftKey ? document.activeElement === first : document.activeElement === last)) {
          event.preventDefault();
          (event.shiftKey ? last : first).focus();
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      if (opener.current?.isConnected) opener.current.focus({ preventScroll: true });
    };
  }, []);

  return (
    <div
      className={standalone ? "add-window" : "backdrop"}
      onMouseDown={(event) => {
        if (!standalone && event.target === event.currentTarget) onClose();
      }}
    >
      <div ref={dialog} tabIndex={-1} className={`dialog${wide ? " wide" : ""}${standalone ? " window-dialog" : ""}`} role="dialog" aria-modal={standalone ? undefined : true} aria-label={title}>
        <header className="dialog-header" data-tauri-drag-region={standalone ? true : undefined}>
          <h2 data-tauri-drag-region={standalone ? true : undefined}>{title}</h2>
          <button type="button" className="icon-button" onClick={onClose} aria-label={t("close")}>
            <Icon name="x" />
          </button>
        </header>
        <div className="dialog-body">{children}</div>
        {footer && <footer className="dialog-footer">{footer}</footer>}
      </div>
    </div>
  );
}
