import { useEffect, useRef, useState } from "react";

import { type AppInfo, type Browser, type InstallInfo, type Settings, api } from "../api";
import { copyText, errorText, useI18n } from "../context";
import { Icon } from "../icons";
import { Dialog } from "./Dialog";

interface Props {
  info: AppInfo | null;
  onClose: () => void;
  onError: (message: string) => void;
  firstRun?: boolean;
  onSetupCompleted: (settings: Settings) => void;
}

export function BrowsersDialog({ info, onClose, onError, firstRun = false, onSetupCompleted }: Props) {
  const { t } = useI18n();
  const [browsers, setBrowsers] = useState<Browser[] | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [results, setResults] = useState<Record<string, InstallInfo>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [active, setActive] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [inspected, setInspected] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const cancel = useRef(false);
  const working = useRef(false);
  const installed = inspected ? results[inspected] : null;

  useEffect(() => {
    let alive = true;
    const load = () =>
      api
        .listBrowsers()
        .then((list) => alive && setBrowsers(list))
        .catch((error) => onError(errorText(error)));
    load();
    const timer = setInterval(load, 3000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [onError]);

  async function install(ids: string[]) {
    if (working.current) return;
    working.current = true;
    cancel.current = false;
    setBusy(true);
    setCopied(false);
    for (const id of ids) {
      if (cancel.current) break;
      setActive(id);
      setErrors((before) => { const next = { ...before }; delete next[id]; return next; });
      try {
        const result = await api.installExtension(id);
        setResults((before) => ({ ...before, [id]: result }));
        setInspected(id);
      } catch (error) {
        setErrors((before) => ({ ...before, [id]: errorText(error) }));
      }
    }
    setActive(null);
    working.current = false;
    setBusy(false);
  }

  async function finish() {
    if (working.current) { cancel.current = true; return; }
    if (firstRun) {
      setBusy(true);
      working.current = true;
      try { onSetupCompleted(await api.completeBrowserSetup()); }
      catch (error) { onError(errorText(error)); working.current = false; setBusy(false); return; }
    }
    onClose();
  }

  function toggle(id: string) {
    setSelected((before) => before.includes(id) ? before.filter((value) => value !== id) : [...before, id]);
  }

  async function copyPath(path: string) {
    await copyText(path);
    setCopied(true);
  }

  return (
    <Dialog
      title={t(firstRun ? "browserSetupTitle" : "browsersTitle")}
      onClose={() => void finish()}
      wide
      footer={
        <>
          <button type="button" className="button" onClick={() => void finish()}>
            {t(busy ? "browserSetupStop" : firstRun && Object.keys(results).length === 0 ? "browserSetupLater" : "close")}
          </button>
          <button type="button" className="button primary" disabled={busy || selected.length === 0} onClick={() => void install(selected)}>
            {t("browserSetupSelected", { count: selected.length })}
          </button>
        </>
      }
    >
      <p className="intro">{t(firstRun ? "browserSetupIntro" : "browsersIntro")}</p>
      {browsers === null && <p role="status" className="muted">{t("browserSetupDetecting")}</p>}
      {browsers && browsers.length > 1 && (
        <label className="browser-select-all">
          <input type="checkbox" disabled={busy} checked={selected.length === browsers.length} onChange={(event) => setSelected(event.target.checked ? browsers.map((browser) => browser.id) : [])} />
          {t("browserSetupSelectAll")}
        </label>
      )}
      {info?.connectionError && (
        <p className="hint warning">
          <Icon name="alert" size={14} /> {t("browserConnectionError")}
          <button type="button" className="button" onClick={() => void api.retryBrowserConnection().catch((error) => onError(errorText(error)))}>{t("browserConnectionRetry")}</button>
        </p>
      )}

      {browsers?.length === 0 && <p className="muted">{t("noBrowsers")}</p>}
      <ul className="browser-list">
        {browsers?.map((browser) => (
          <li key={browser.id} className="browser">
            <input type="checkbox" aria-label={t("browserSetupChoose", { browser: browser.name })} disabled={busy} checked={selected.includes(browser.id)} onChange={() => toggle(browser.id)} />
            <span className={`browser-badge ${browser.id}`} aria-hidden="true">
              {browser.name.slice(0, 1)}
            </span>
            <div className="browser-name">
              <strong>{browser.name}</strong>
              <span className={browser.connected ? "status-ok" : "muted"}>
                {active === browser.id ? t("browserSetupWorking") : errors[browser.id] ? errors[browser.id] : browser.connected ? (
                  <>
                    <Icon name="check" size={13} /> {t("connected")}
                  </>
                ) : (
                  results[browser.id] ? t(results[browser.id].method === "automatic" ? "browserSetupLoaded" : "browserSetupConfirm") : t("notConnected")
                )}
              </span>
            </div>
            <button
              type="button"
              className={browser.connected ? "button" : "button primary"}
              disabled={busy}
              onClick={() => results[browser.id] ? setInspected(browser.id) : void install([browser.id])}
            >
              {results[browser.id] ? t("browserSetupSteps") : browser.connected ? t("reinstall") : t("install")}
            </button>
          </li>
        ))}
      </ul>

      {installed && (
        <section className="steps" aria-live="polite">
          <strong>{installed.browser}</strong>
          {installed.method === "automatic" ? (
            <p>{t("browserSetupAutomatic", { browser: installed.browser })}</p>
          ) : installed.method === "store" ? (
            <p>{t("storeSteps", { browser: installed.browser })}</p>
          ) : (
            <>
              <p>{t("manualIntro", { browser: installed.browser })}</p>
              <ol>
                {installed.engine === "chromium" ? (
                  <>
                    <li>{t("chromiumStep1")}</li>
                    <li>
                      {t("chromiumStep2")}
                      <PathBox path={installed.extensionDir} copied={copied} onCopy={copyPath} engine="chromium" />
                    </li>
                    <li>{t("chromiumStep3")}</li>
                  </>
                ) : (
                  <>
                    <li>{t("firefoxStep1")}</li>
                    <li>
                      {t("firefoxStep2")}
                      <PathBox path={installed.extensionDir} copied={copied} onCopy={copyPath} engine="firefox" />
                    </li>
                  </>
                )}
              </ol>
            </>
          )}
          {installed.engine === "firefox" && <p className="muted">{t("firefoxNote")}</p>}
        </section>
      )}
    </Dialog>
  );
}

function PathBox(props: {
  path: string;
  copied: boolean;
  engine: InstallInfo["engine"];
  onCopy: (path: string) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="path-box">
      <code>{props.path}</code>
      <div className="path-actions">
        <button type="button" className="button small" onClick={() => props.onCopy(props.path)}>
          <Icon name="copy" size={14} />
          {props.copied ? t("copied") : t("copyPath")}
        </button>
        <button type="button" className="button small" onClick={() => api.openExtensionFolder(props.engine)}>
          <Icon name="folder" size={14} />
          {t("openFolder")}
        </button>
      </div>
    </div>
  );
}
