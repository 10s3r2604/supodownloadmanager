// Gives an unfinished download a new link (the old one expired, say) while
// keeping what was already downloaded.

import { useState } from "react";

import { type Download, api } from "../api";
import { errorText, useI18n } from "../context";
import { Icon } from "../icons";
import { Dialog } from "./Dialog";

interface Props {
  download: Download;
  onClose: () => void;
  onDone: (message: string) => void;
}

export function AddressDialog({ download, onClose, onDone }: Props) {
  const { t } = useI18n();
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const valid = /^https?:\/\/\S+$/i.test(url.trim());

  async function submit() {
    if (!valid || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.changeAddress(download.id, url.trim());
      onDone(t("addressChanged", { name: download.fileName }));
      onClose();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  }

  return (
    <Dialog
      title={t("addressTitle")}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            {t("cancel")}
          </button>
          <button type="button" className="button primary" disabled={!valid || busy} onClick={submit}>
            {busy ? t("checking") : t("addressUse")}
          </button>
        </>
      }
    >
      <form
        className="form"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <p className="muted">{t("addressIntro", { name: download.fileName })}</p>
        <label className="field">
          <span>{t("addressOld")}</span>
          <input value={download.url} readOnly spellCheck={false} />
        </label>
        <label className="field">
          <span>{t("addressNew")}</span>
          <input autoFocus value={url} spellCheck={false} placeholder="https://…" onChange={(e) => setUrl(e.target.value)} />
        </label>
        {error && (
          <p className="hint warning">
            <Icon name="alert" size={14} /> {error}
          </p>
        )}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}
