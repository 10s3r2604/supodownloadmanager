import { useEffect, useState } from "react";

import { type AddPrompt, type ProbeResult, type Settings, api } from "../api";
import { errorText, useI18n } from "../context";
import { formatBytes } from "../format";
import { Icon, categoryIcon } from "../icons";
import { Dialog } from "./Dialog";

interface Props {
  prompt?: AddPrompt;
  initialUrl?: string;
  settings: Settings;
  onClose: () => void;
  onError: (message: string) => void;
}

const isLink = (text: string) => /^https?:\/\/\S+$/i.test(text.trim());

export function AddDialog({ prompt, initialUrl, settings, onClose, onError }: Props) {
  const { t, language } = useI18n();
  const headers = prompt?.headers ?? [];
  const [url, setUrl] = useState(prompt?.url ?? initialUrl ?? "");
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const [probing, setProbing] = useState(false);
  const [probeError, setProbeError] = useState<string | null>(null);
  const [fileName, setFileName] = useState(prompt?.fileName ?? "");
  const [nameEdited, setNameEdited] = useState(false);
  const [directory, setDirectory] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const valid = isLink(url);

  useEffect(() => {
    setProbe(null);
    setProbeError(null);
    if (!isLink(url)) {
      setProbing(false);
      return;
    }
    let cancelled = false;
    setProbing(true);
    const timer = setTimeout(async () => {
      try {
        const result = await api.probe(url.trim(), headers);
        if (cancelled) return;
        setProbe(result);
        if (!nameEdited) setFileName(result.fileName);
      } catch (error) {
        if (!cancelled) setProbeError(errorText(error));
      } finally {
        if (!cancelled) setProbing(false);
      }
    }, 350);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
    // Only the link decides what to ask the server.
  }, [url]);

  const shownDirectory = directory ?? probe?.directory ?? settings.downloadDir;

  async function browse() {
    const folder = await api.pickFolder();
    if (folder) setDirectory(folder);
  }

  async function submit(start: boolean) {
    if (!valid || busy) return;
    setBusy(true);
    try {
      await api.add({
        url: url.trim(),
        headers,
        fileName: fileName.trim() || undefined,
        directory: directory ?? undefined,
        start,
      });
      onClose();
    } catch (error) {
      onError(errorText(error));
      setBusy(false);
    }
  }

  return (
    <Dialog
      title={t("addTitle")}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            {t("cancel")}
          </button>
          <button type="button" className="button" disabled={!valid || busy} onClick={() => submit(false)}>
            {t("downloadLater")}
          </button>
          <button type="button" className="button primary" disabled={!valid || busy} onClick={() => submit(true)}>
            <Icon name="download" size={16} />
            {t("downloadNow")}
          </button>
        </>
      }
    >
      <form
        className="form"
        onSubmit={(event) => {
          event.preventDefault();
          submit(true);
        }}
      >
        <label className="field">
          <span>{t("urlLabel")}</span>
          <input
            autoFocus
            value={url}
            placeholder="https://"
            spellCheck={false}
            onChange={(event) => setUrl(event.target.value)}
          />
        </label>
        {url.trim() && !valid && <p className="hint warning">{t("invalidUrl")}</p>}

        <div className="probe">
          {probing && <span className="muted">{t("checking")}</span>}
          {probeError && (
            <span className="warning">
              <Icon name="alert" size={14} /> {t("probeFailed", { error: probeError })}
            </span>
          )}
          {probe && !probing && (
            <>
              <span className="chip">
                <Icon name={categoryIcon[probe.category]} size={14} />
                {t(`cat.${probe.category}`)}
              </span>
              <span className="chip">{probe.size == null ? t("unknownSize") : formatBytes(probe.size, language)}</span>
              <span className={`chip ${probe.resumable ? "good" : "warn"}`}>
                {probe.resumable ? t("resumableYes") : t("resumableNo")}
              </span>
            </>
          )}
          {headers.length > 0 && (
            <span className="chip">
              <Icon name="globe" size={14} />
              {t("fromBrowser")}
            </span>
          )}
        </div>

        <label className="field">
          <span>{t("fileName")}</span>
          <input
            value={fileName}
            spellCheck={false}
            onChange={(event) => {
              setFileName(event.target.value);
              setNameEdited(true);
            }}
          />
        </label>

        <div className="field">
          <span>{t("saveIn")}</span>
          <div className="input-row">
            <input value={shownDirectory} spellCheck={false} onChange={(event) => setDirectory(event.target.value)} />
            <button type="button" className="button" onClick={browse}>
              <Icon name="folder" size={16} />
              {t("browse")}
            </button>
          </div>
        </div>
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}
