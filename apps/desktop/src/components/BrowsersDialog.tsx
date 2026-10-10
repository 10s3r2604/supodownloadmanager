import { useEffect, useState } from "react";

import { type AppInfo, type Browser, type InstallInfo, api } from "../api";
import { copyText, errorText, useI18n } from "../context";
import { Icon } from "../icons";
import { Dialog } from "./Dialog";

interface Props {
  info: AppInfo | null;
  onClose: () => void;
  onError: (message: string) => void;
}

export function BrowsersDialog({ info, onClose, onError }: Props) {
  const { t } = useI18n();
  const [browsers, setBrowsers] = useState<Browser[] | null>(null);
  const [installed, setInstalled] = useState<InstallInfo | null>(null);
  const [copied, setCopied] = useState(false);

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

  async function install(id: string) {
    try {
      setCopied(false);
      setInstalled(await api.installExtension(id));
    } catch (error) {
      onError(errorText(error));
    }
  }

  async function copyPath(path: string) {
    await copyText(path);
    setCopied(true);
  }

  return (
    <Dialog
      title={t("browsersTitle")}
      onClose={onClose}
      wide
      footer={
        <button type="button" className="button" onClick={onClose}>
          {t("close")}
        </button>
      }
    >
      <p className="intro">{t("browsersIntro")}</p>
      {info?.bridgeError && (
        <p className="hint warning">
          <Icon name="alert" size={14} /> {t("bridgeError", { error: info.bridgeError })}
        </p>
      )}

      {browsers?.length === 0 && <p className="muted">{t("noBrowsers")}</p>}
      <ul className="browser-list">
        {browsers?.map((browser) => (
          <li key={browser.id} className="browser">
            <span className={`browser-badge ${browser.id}`} aria-hidden="true">
              {browser.name.slice(0, 1)}
            </span>
            <div className="browser-name">
              <strong>{browser.name}</strong>
              <span className={browser.connected ? "status-ok" : "muted"}>
                {browser.connected ? (
                  <>
                    <Icon name="check" size={13} /> {t("connected")}
                  </>
                ) : (
                  t("notConnected")
                )}
              </span>
            </div>
            <button
              type="button"
              className={browser.connected ? "button" : "button primary"}
              onClick={() => install(browser.id)}
            >
              {browser.connected ? t("reinstall") : t("install")}
            </button>
          </li>
        ))}
      </ul>

      {installed && (
        <section className="steps" aria-live="polite">
          {installed.method === "store" ? (
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
              {installed.engine === "firefox" && <p className="muted">{t("firefoxNote")}</p>}
            </>
          )}
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
