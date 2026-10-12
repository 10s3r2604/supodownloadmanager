import { useEffect, useState } from "react";

import { type AddPrompt, type Queue, type Settings, api } from "./api";
import { useAppearance } from "./appearance";
import { AddDialog } from "./components/AddDialog";
import { Dialog } from "./components/Dialog";
import { I18nContext, errorText } from "./context";

/** A browser download confirmation in its own native window. */
export function AddWindow() {
  const [data, setData] = useState<{ prompt: AddPrompt; settings: Settings; queues: Queue[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const { i18n } = useAppearance(data?.settings ?? null);
  const { t } = i18n;
  const close = () => { void api.closeWindow(); };

  useEffect(() => {
    let active = true;
    setError(null);
    Promise.all([api.getAddPrompt(), api.getSettings(), api.listQueues()])
      .then(([prompt, settings, queues]) => {
        if (active) setData({ prompt, settings, queues });
      })
      .catch((error) => { if (active) setError(errorText(error)); });
    return () => { active = false; };
  }, [attempt]);

  useEffect(() => {
    // Paint useful content before revealing the WebView2 window.
    if (data || error) void api.showWindow().catch(console.error);
  }, [data, error]);

  return (
    <I18nContext.Provider value={i18n}>
      {data ? (
        <AddDialog
          standalone
          prompt={data.prompt}
          settings={data.settings}
          queues={data.queues}
          error={error}
          onError={setError}
          onClose={close}
        />
      ) : (
        <Dialog standalone title={t("addTitle")} onClose={close} footer={
          <>
            <button className="button" onClick={close}>{t("cancel")}</button>
            {error && <button className="button primary" onClick={() => setAttempt((value) => value + 1)}>{t("actRetry")}</button>}
          </>
        }>
          <p className={error ? "warning" : "muted"} role={error ? "alert" : "status"}>{error ?? t("checking")}</p>
        </Dialog>
      )}
    </I18nContext.Provider>
  );
}
