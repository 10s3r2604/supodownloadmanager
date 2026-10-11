import { useI18n } from "../context";
import { Icon } from "../icons";

export interface Toast {
  id: number;
  kind: "error" | "clipboard" | "info";
  text: string;
  url?: string;
}

interface Props {
  toasts: Toast[];
  onDismiss: (id: number) => void;
  onDownload: (url: string) => void;
}

export function Toasts({ toasts, onDismiss, onDownload }: Props) {
  const { t } = useI18n();
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast ${toast.kind}`} role="status">
          <Icon name={toast.kind === "error" ? "alert" : toast.kind === "info" ? "tick" : "link"} size={16} />
          <div className="toast-text">
            <span>{toast.text}</span>
            {toast.url && <small title={toast.url}>{toast.url}</small>}
          </div>
          {toast.url && (
            <button
              type="button"
              className="button small primary"
              onClick={() => {
                onDownload(toast.url!);
                onDismiss(toast.id);
              }}
            >
              {t("download")}
            </button>
          )}
          <button type="button" className="icon-button" aria-label={t("dismiss")} onClick={() => onDismiss(toast.id)}>
            <Icon name="x" size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
