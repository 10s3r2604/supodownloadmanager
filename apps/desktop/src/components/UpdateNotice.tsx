import { useEffect, useRef, useState } from "react";

import { api, type UpdateState } from "../api";
import { errorText, useI18n } from "../context";
import { formatBytes, formatDate } from "../format";
import type { TextKey } from "../i18n";
import { Icon } from "../icons";
import "./update-notice.css";

interface Props {
  /** Settings shows the installed version and manual check even when no update is available. */
  inSettings?: boolean;
  /** Restarting must never silently discard an unsaved Settings draft. */
  installBlocked?: boolean;
  onSupportChange?: (supported: boolean) => void;
}

const UPDATE_ERRORS: Record<string, TextKey> = {
  network: "updateErrorNetwork",
  manifest: "updateErrorManifest",
  signature: "updateErrorSignature",
  storage: "updateErrorStorage",
  installer: "updateErrorInstaller",
  timeout: "updateErrorTimeout",
  unsupported: "updateUnsupported",
  transport: "updateErrorTransport",
};

/** An update never opens a modal or interrupts editing or downloads. */
export function UpdateNotice({ inSettings = false, installBlocked = false, onSupportChange }: Props) {
  const { t, language } = useI18n();
  const [state, setState] = useState<UpdateState | null>(null);
  const [requestError, setRequestError] = useState<string | null>(null);
  const [readError, setReadError] = useState<string | null>(null);
  const [acting, setActing] = useState<"check" | "install" | null>(null);
  const [hidden, setHidden] = useState<string | null>(null);
  const mounted = useRef(false);
  const actionPending = useRef(false);
  const generation = useRef(0);

  useEffect(() => {
    mounted.current = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      if (!actionPending.current) {
        const current = ++generation.current;
        try {
          const next = await api.getUpdateState();
          if (mounted.current && current === generation.current) {
            setState(next);
            setReadError(null);
          }
        } catch {
          if (mounted.current && current === generation.current) setReadError("transport");
        }
      }
      if (mounted.current) timer = setTimeout(poll, 2_000);
    };
    void poll();
    return () => {
      mounted.current = false;
      ++generation.current;
      if (timer !== undefined) clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    if (state) onSupportChange?.(state.supported);
  }, [state?.supported, onSupportChange]);

  async function act(kind: "check" | "install") {
    if (actionPending.current || (kind === "install" && installBlocked)) return;
    actionPending.current = true;
    ++generation.current;
    setActing(kind);
    setRequestError(null);
    setReadError(null);
    try {
      const next = await (kind === "check" ? api.checkForUpdates() : api.installUpdate());
      if (mounted.current) setState(next);
    } catch (error) {
      if (mounted.current) setRequestError(errorText(error));
    } finally {
      actionPending.current = false;
      if (mounted.current) setActing(null);
    }
  }

  const phase = state?.phase ?? "idle";
  const error = requestError ?? readError ?? state?.error;
  const key = `${phase}:${state?.version ?? ""}:${error ?? ""}`;
  const busy = acting !== null || phase === "checking" || phase === "downloading" || phase === "installing";
  const ready = phase === "ready" || phase === "waiting";
  const supported = state?.supported === true;
  const visible = Boolean(error) || ready || busy;
  if (!inSettings && ((!supported && !error) || !visible || hidden === key)) return null;

  let message: TextKey;
  if (error) message = UPDATE_ERRORS[error] ?? "updateFailed";
  else if (acting === "check") message = "updateChecking";
  else if (acting === "install") message = "updateInstalling";
  else if (state && !supported) message = "updateUnsupported";
  else if (phase === "disabled") message = "updateDisabled";
  else if (phase === "idle") message = state?.lastChecked ? "updateUpToDate" : "updateIdle";
  else {
    const messages: Partial<Record<UpdateState["phase"], TextKey>> = {
      checking: "updateChecking",
      downloading: "updateDownloading",
      ready: "updateReady",
      waiting: "updateWaiting",
      installing: "updateInstalling",
      error: "updateFailed",
    };
    message = messages[phase] ?? "updateIdle";
  }

  return (
    <section
      className={`update-notice${inSettings ? " settings-update" : " floating-update"}${error ? " update-error" : ""}`}
      aria-label={t("updateTitle")}
    >
      {inSettings && state && (
        <div className="update-version">
          <strong>{t("updateVersion", { version: state.currentVersion })}</strong>
          {state.lastChecked != null && (
            <small className="muted">{t("updateLastChecked", { time: formatDate(state.lastChecked, language) })}</small>
          )}
        </div>
      )}
      <div className="update-message" role={error ? "alert" : "status"}>
        <Icon name={error ? "alert" : ready ? "download" : "retry"} size={16} />
        <span>{t(message, { version: state?.version ?? "" })}</span>
      </div>
      {phase === "downloading" && state && (
        <div className="update-download">
          <progress aria-label={t("updateDownloading", { version: state.version ?? "" })} value={state.total ? state.downloaded : undefined} max={state.total ?? 1} />
          <small className="muted">
            {formatBytes(state.downloaded, language)}{state.total != null ? ` / ${formatBytes(state.total, language)}` : ""}
          </small>
        </div>
      )}
      {inSettings && ready && (
        <small className="muted">{t(installBlocked ? "updateSaveFirst" : "updateInstallHint")}</small>
      )}
      <div className="update-actions">
        {(inSettings || Boolean(error)) && (
          <button type="button" className="button small" disabled={busy || !supported} onClick={() => void act("check")}>
            {t("updateCheckNow")}
          </button>
        )}
        {ready && !error && (
          <button
            type="button"
            className="button small primary"
            disabled={busy || !supported || installBlocked}
            title={installBlocked ? t("updateSaveFirst") : t("updateInstallHint")}
            onClick={() => void act("install")}
          >
            {t("updateInstall")}
          </button>
        )}
        {!inSettings && (
          <button type="button" className="icon-button" aria-label={t("updateHide")} onClick={() => setHidden(key)}>
            <Icon name="x" size={16} />
          </button>
        )}
      </div>
    </section>
  );
}
