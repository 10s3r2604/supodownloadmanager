import { useState } from "react";

import { useI18n } from "../context";
import { Dialog } from "./Dialog";

interface Props {
  count: number;
  hasFinished: boolean;
  onClose: () => void;
  onConfirm: (deleteFiles: boolean) => void;
}

export function DeleteDialog({ count, hasFinished, onClose, onConfirm }: Props) {
  const { t } = useI18n();
  const [deleteFiles, setDeleteFiles] = useState(false);
  return (
    <Dialog
      title={t("deleteTitle")}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            {t("cancel")}
          </button>
          <button type="button" className="button danger" autoFocus onClick={() => onConfirm(deleteFiles)}>
            {t("delete")}
          </button>
        </>
      }
    >
      <p>{t("deleteBody", { n: count })}</p>
      {hasFinished && (
        <label className="check">
          <input type="checkbox" checked={deleteFiles} onChange={(e) => setDeleteFiles(e.target.checked)} />
          {t("deleteFiles")}
        </label>
      )}
    </Dialog>
  );
}
