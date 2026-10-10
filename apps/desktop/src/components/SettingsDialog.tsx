import { useState } from "react";

import { type Settings, api } from "../api";
import { errorText, useI18n } from "../context";
import { Icon } from "../icons";
import { Dialog } from "./Dialog";

interface Props {
  settings: Settings;
  onClose: () => void;
  onSaved: (settings: Settings) => void;
  onError: (message: string) => void;
}

type Tab = "general" | "connection" | "browser";
type ProxyMode = "system" | "none" | "custom";

export function SettingsDialog({ settings, onClose, onSaved, onError }: Props) {
  const { t } = useI18n();
  const [tab, setTab] = useState<Tab>("general");
  const [draft, setDraft] = useState<Settings>(settings);
  const [proxyMode, setProxyMode] = useState<ProxyMode>(
    settings.proxy === "" ? "system" : settings.proxy === "none" ? "none" : "custom",
  );
  const [customProxy, setCustomProxy] = useState(proxyMode === "custom" ? settings.proxy : "");
  const [types, setTypes] = useState(settings.captureExtensions.join(", "));
  const [saving, setSaving] = useState(false);

  const set = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));

  async function save() {
    setSaving(true);
    const proxy = proxyMode === "system" ? "" : proxyMode === "none" ? "none" : customProxy.trim();
    const captureExtensions = types
      .split(/[,\s]+/)
      .map((ext) => ext.trim().replace(/^\./, "").toLowerCase())
      .filter(Boolean);
    try {
      onSaved(await api.saveSettings({ ...draft, proxy, captureExtensions }));
      onClose();
    } catch (error) {
      onError(errorText(error));
      setSaving(false);
    }
  }

  async function browse() {
    const folder = await api.pickFolder();
    if (folder) set("downloadDir", folder);
  }

  const number = (value: string, min: number, max: number) =>
    Math.min(max, Math.max(min, Math.round(Number(value) || 0)));

  return (
    <Dialog
      title={t("settingsTitle")}
      onClose={onClose}
      wide
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            {t("cancel")}
          </button>
          <button type="button" className="button primary" disabled={saving} onClick={save}>
            {t("save")}
          </button>
        </>
      }
    >
      <div className="tabs" role="tablist">
        {(["general", "connection", "browser"] as const).map((name) => (
          <button
            key={name}
            type="button"
            role="tab"
            aria-selected={tab === name}
            className={tab === name ? "tab active" : "tab"}
            onClick={() => setTab(name)}
          >
            {t(name === "general" ? "tabGeneral" : name === "connection" ? "tabConnection" : "tabBrowser")}
          </button>
        ))}
      </div>

      {tab === "general" && (
        <div className="form">
          <div className="field">
            <span>{t("downloadFolder")}</span>
            <div className="input-row">
              <input value={draft.downloadDir} onChange={(e) => set("downloadDir", e.target.value)} />
              <button type="button" className="button" onClick={browse}>
                <Icon name="folder" size={16} />
                {t("browse")}
              </button>
            </div>
          </div>
          <Check label={t("categoryFolders")} checked={draft.categoryFolders} onChange={(v) => set("categoryFolders", v)} />
          <div className="field-grid">
            <label className="field">
              <span>{t("language")}</span>
              <select value={draft.language} onChange={(e) => set("language", e.target.value as Settings["language"])}>
                <option value="auto">{t("sameAsSystem")}</option>
                <option value="en">English</option>
                <option value="vi">Tiếng Việt</option>
              </select>
            </label>
            <label className="field">
              <span>{t("theme")}</span>
              <select value={draft.theme} onChange={(e) => set("theme", e.target.value as Settings["theme"])}>
                <option value="system">{t("sameAsSystem")}</option>
                <option value="light">{t("themeLight")}</option>
                <option value="dark">{t("themeDark")}</option>
              </select>
            </label>
          </div>
          <Check label={t("closeToTray")} checked={draft.closeToTray} onChange={(v) => set("closeToTray", v)} />
          <Check label={t("notifications")} checked={draft.notifications} onChange={(v) => set("notifications", v)} />
        </div>
      )}

      {tab === "connection" && (
        <div className="form">
          <div className="field-grid">
            <label className="field">
              <span>{t("connectionsPerFile")}</span>
              <input
                type="number"
                min={1}
                max={32}
                value={draft.connections}
                onChange={(e) => set("connections", number(e.target.value, 1, 32))}
              />
            </label>
            <label className="field">
              <span>{t("maxParallel")}</span>
              <input
                type="number"
                min={1}
                max={16}
                value={draft.maxParallel}
                onChange={(e) => set("maxParallel", number(e.target.value, 1, 16))}
              />
            </label>
          </div>
          <label className="field">
            <span>{t("speedLimit")}</span>
            <input
              type="number"
              min={0}
              value={Math.round(draft.speedLimit / 1024)}
              onChange={(e) => set("speedLimit", number(e.target.value, 0, 10_000_000) * 1024)}
            />
            <small className="muted">{t("speedLimitHint")}</small>
          </label>
          <fieldset className="field">
            <span>{t("proxy")}</span>
            <div className="radios">
              {(["system", "none", "custom"] as const).map((mode) => (
                <label key={mode} className="radio">
                  <input type="radio" checked={proxyMode === mode} onChange={() => setProxyMode(mode)} />
                  {t(mode === "system" ? "proxySystem" : mode === "none" ? "proxyNone" : "proxyCustom")}
                </label>
              ))}
            </div>
            {proxyMode === "custom" && (
              <input
                value={customProxy}
                placeholder="http://127.0.0.1:8080 · socks5://127.0.0.1:1080"
                spellCheck={false}
                onChange={(e) => setCustomProxy(e.target.value)}
              />
            )}
          </fieldset>
        </div>
      )}

      {tab === "browser" && (
        <div className="form">
          <Check
            label={t("browserIntegration")}
            checked={draft.browserIntegration}
            onChange={(v) => set("browserIntegration", v)}
          />
          <Check
            label={t("confirmBrowser")}
            checked={draft.confirmBrowserDownloads}
            disabled={!draft.browserIntegration}
            onChange={(v) => set("confirmBrowserDownloads", v)}
          />
          <Check label={t("clipboardMonitor")} checked={draft.clipboardMonitor} onChange={(v) => set("clipboardMonitor", v)} />
          <label className="field">
            <span>{t("captureTypes")}</span>
            <textarea rows={3} value={types} spellCheck={false} onChange={(e) => setTypes(e.target.value)} />
            <small className="muted">{t("captureTypesHint")}</small>
          </label>
          <label className="field narrow">
            <span>{t("port")}</span>
            <input
              type="number"
              min={1024}
              max={65535}
              value={draft.bridgePort}
              onChange={(e) => set("bridgePort", number(e.target.value, 1024, 65535))}
            />
            <small className="muted">{t("portHint")}</small>
          </label>
        </div>
      )}
    </Dialog>
  );
}

function Check(props: { label: string; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void }) {
  return (
    <label className={`check${props.disabled ? " disabled" : ""}`}>
      <input
        type="checkbox"
        checked={props.checked}
        disabled={props.disabled}
        onChange={(e) => props.onChange(e.target.checked)}
      />
      {props.label}
    </label>
  );
}
