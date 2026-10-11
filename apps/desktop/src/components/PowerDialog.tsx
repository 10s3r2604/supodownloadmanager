// Counts down before SDM quits or the computer sleeps or turns off, so the
// user can still stop it.

import { useEffect, useState } from "react";

import { type PowerCountdown, api } from "../api";
import { errorText, useI18n } from "../context";
import type { TextKey } from "../i18n";
import { Icon } from "../icons";
import { Dialog } from "./Dialog";

export function PowerDialog({ countdown, onError }: { countdown: PowerCountdown; onError: (message: string) => void }) {
  const { t } = useI18n();
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.ceil((countdown.at - now) / 1000));
  const cancel = () => api.cancelPowerAction().catch((error) => onError(errorText(error)));

  return (
    <Dialog
      title={t("powerTitle")}
      onClose={cancel}
      footer={
        <button type="button" className="button primary" autoFocus onClick={cancel}>
          {t("cancel")}
        </button>
      }
    >
      <div className="power">
        <span className="power-count" aria-hidden="true">
          <Icon name="power" size={22} />
          {seconds}
        </span>
        <div>
          <p>{countdown.queue ? t("powerReasonQueue", { queue: countdown.queue }) : t("powerReasonAll")}</p>
          <p role="timer">
            <strong>{t(`powerIn.${countdown.action}` as TextKey, { s: seconds })}</strong>
          </p>
        </div>
      </div>
    </Dialog>
  );
}
