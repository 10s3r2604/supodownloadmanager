import { useEffect, useState } from "react";

import { type AppInfo, type ProgressStyle, SEED_FOREVER, type Settings, type Skin, api } from "../api";
import { errorText, useI18n } from "../context";
import type { TextKey } from "../i18n";
import { Icon } from "../icons";
import { Dialog } from "./Dialog";
import { UpdateNotice } from "./UpdateNotice";

interface Props {
  settings: Settings;
  /** Called with the draft while the look is being changed, so it shows right away. */
  onPreview: (settings: Settings | null) => void;
  onClose: () => void;
  onSaved: (settings: Settings) => void;
  onError: (message: string) => void;
}

type Tab = "appearance" | "general" | "connection" | "browser" | "torrent" | "updates";
type ProxyMode = "system" | "none" | "custom";

const PROGRESS_STYLES: { style: ProgressStyle; label: TextKey; hint: TextKey }[] = [
  { style: "graph", label: "styleGraph", hint: "styleGraphHint" },
  { style: "ring", label: "styleRing", hint: "styleRingHint" },
  { style: "classic", label: "styleClassic", hint: "styleClassicHint" },
];

const TABS: { name: Tab; label: TextKey }[] = [
  { name: "appearance", label: "appearance" },
  { name: "general", label: "tabGeneral" },
  { name: "connection", label: "tabConnection" },
  { name: "torrent", label: "sectionTorrent" },
  { name: "browser", label: "tabBrowser" },
  { name: "updates", label: "tabUpdates" },
];

const SEED_CHOICES: { value: number; label: TextKey }[] = [
  { value: 0, label: "seedNone" },
  { value: 100, label: "seedOne" },
  { value: 200, label: "seedTwo" },
  { value: SEED_FOREVER, label: "seedForever" },
];

export function SettingsDialog({ settings, onPreview, onClose, onSaved, onError }: Props) {
  const { t } = useI18n();
  const [tab, setTab] = useState<Tab>("appearance");
  const [draft, setDraft] = useState<Settings>(settings);
  const [proxyMode, setProxyMode] = useState<ProxyMode>(
    settings.proxy === "" ? "system" : settings.proxy === "none" ? "none" : "custom",
  );
  const [customProxy, setCustomProxy] = useState(proxyMode === "custom" ? settings.proxy : "");
  const [types, setTypes] = useState(settings.captureExtensions.join(", "));
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [updatesSupported, setUpdatesSupported] = useState<boolean | null>(null);
  const [browserInfo, setBrowserInfo] = useState<AppInfo | null>(null);
  const [browserConnected, setBrowserConnected] = useState(false);
  const [checkingConnection, setCheckingConnection] = useState(false);
  const [connectionFailed, setConnectionFailed] = useState(false);

  useEffect(() => {
    let live = true;
    if (tab !== "browser") return;
    setCheckingConnection(true);
    Promise.all([api.appInfo(), api.listBrowsers()]).then(([info, browsers]) => {
      if (live) { setBrowserInfo(info); setBrowserConnected(browsers.some((browser) => browser.connected)); setConnectionFailed(false); }
    }).catch(() => { if (live) setConnectionFailed(true); }).finally(() => { if (live) setCheckingConnection(false); });
    return () => { live = false; };
  }, [tab]);

  async function retryConnection() {
    setCheckingConnection(true);
    try {
      await api.retryBrowserConnection();
      const [info, browsers] = await Promise.all([api.appInfo(), api.listBrowsers()]);
      setBrowserInfo(info); setBrowserConnected(browsers.some((browser) => browser.connected)); setConnectionFailed(false);
    } catch { setConnectionFailed(true); }
    finally { setCheckingConnection(false); }
  }

  const set = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));

  // Try the look out while choosing it.
  const look = (patch: Partial<Settings>) => {
    const next = { ...draft, ...patch };
    setDraft(next);
    onPreview(next);
  };

  const proxy = proxyMode === "system" ? "" : proxyMode === "none" ? "none" : customProxy.trim();
  const captureExtensions = types
    .split(/[,\s]+/)
    .map((ext) => ext.trim().replace(/^\./, "").toLowerCase())
    .filter(Boolean);
  const effectiveDraft = { ...draft, proxy, captureExtensions };
  const dirty = JSON.stringify(effectiveDraft) !== JSON.stringify(settings);

  async function save() {
    setSaving(true);
    setSaveError(null);
    try {
      onSaved(await api.saveSettings(effectiveDraft));
      onClose();
    } catch (error) {
      setSaveError(errorText(error));
      setSaving(false);
    }
  }

  async function browse(key: "downloadDir" | "tempDir") {
    try {
      const folder = await api.pickFolder();
      if (folder) set(key, folder);
    } catch (error) {
      onError(errorText(error));
    }
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
        {TABS.map(({ name, label }) => (
          <button
            key={name}
            type="button"
            role="tab"
            aria-selected={tab === name}
            className={tab === name ? "tab active" : "tab"}
            onClick={() => setTab(name)}
          >
            {t(label)}
          </button>
        ))}
      </div>

      {saveError && (
        <p className="hint warning" role="alert">
          {t("settingsSaveFailed")} {saveError}
        </p>
      )}

      {tab === "appearance" && (
        <div className="form">
          <div className="skins" role="radiogroup" aria-label={t("appearance")}>
            {(["minimal", "neon"] as Skin[]).map((skin) => (
              <button
                key={skin}
                type="button"
                role="radio"
                aria-checked={draft.skin === skin}
                className={`skin-card${draft.skin === skin ? " on" : ""}`}
                onClick={() => look({ skin })}
              >
                <SkinPreview skin={skin} dark={skin === "minimal" && draft.theme !== "light"} />
                <strong>
                  {t(skin === "neon" ? "skinNeon" : "skinMinimal")}
                  {skin === "minimal" && <em>{t("defaultLabel")}</em>}
                </strong>
                <small>{t(skin === "neon" ? "skinNeonHint" : "skinMinimalHint")}</small>
              </button>
            ))}
          </div>
          <div className="field-grid">
            <label className={`field${draft.skin === "neon" ? " disabled" : ""}`}>
              <span>{t("colorMode")}</span>
              <select
                value={draft.theme}
                disabled={draft.skin === "neon"}
                onChange={(e) => look({ theme: e.target.value as Settings["theme"] })}
              >
                <option value="system">{t("sameAsSystem")}</option>
                <option value="light">{t("themeLight")}</option>
                <option value="dark">{t("themeDark")}</option>
              </select>
            </label>
            <label className="field">
              <span>{t("language")}</span>
              <select value={draft.language} onChange={(e) => look({ language: e.target.value as Settings["language"] })}>
                <option value="auto">{t("sameAsSystem")}</option>
                <option value="en">English</option>
                <option value="vi">Tiếng Việt</option>
              </select>
            </label>
          </div>

          <div className="field">
            <span>{t("progressStyleTitle")}</span>
            <div className="styles" role="radiogroup" aria-label={t("progressStyleTitle")}>
              {PROGRESS_STYLES.map(({ style, label, hint }) => (
                <button
                  key={style}
                  type="button"
                  role="radio"
                  aria-checked={draft.progressStyle === style}
                  className={`style-card${draft.progressStyle === style ? " on" : ""}`}
                  onClick={() => set("progressStyle", style)}
                >
                  <StylePreview style={style} />
                  <strong>
                    {t(label)}
                    {style === "graph" && <em>{t("defaultLabel")}</em>}
                  </strong>
                  <small>{t(hint)}</small>
                </button>
              ))}
            </div>
          </div>
          <Check
            label={t("progressWindowSetting")}
            checked={draft.progressWindow}
            onChange={(v) => set("progressWindow", v)}
          />
          <Check
            label={t("completeDialogSetting")}
            checked={draft.completeDialog}
            onChange={(v) => set("completeDialog", v)}
          />
        </div>
      )}

      {tab === "general" && (
        <div className="form">
          <div className="field">
            <span>{t("downloadFolder")}</span>
            <div className="input-row">
              <input aria-label={t("downloadFolder")} value={draft.downloadDir} onChange={(e) => set("downloadDir", e.target.value)} />
              <button type="button" className="button" onClick={() => browse("downloadDir")}>
                <Icon name="folder" size={16} />
                {t("browse")}
              </button>
            </div>
          </div>
          <div className="field">
            <span>{t("tempFolder")}</span>
            <div className="input-row">
              <input aria-label={t("tempFolder")} value={draft.tempDir} onChange={(e) => set("tempDir", e.target.value)} />
              <button type="button" className="button" onClick={() => browse("tempDir")}>
                <Icon name="folder" size={16} />
                {t("browse")}
              </button>
            </div>
            <small className="hint">{t("tempFolderHint")}</small>
          </div>
          <Check label={t("categoryFolders")} checked={draft.categoryFolders} onChange={(v) => set("categoryFolders", v)} />
          <Check label={t("startWithWindows")} checked={draft.startWithWindows} onChange={(v) => set("startWithWindows", v)} />
          <small className="hint">{t("startWithWindowsHint")}</small>
          <Check label={t("closeToTray")} checked={draft.closeToTray} onChange={(v) => set("closeToTray", v)} />
          <Check label={t("notifications")} checked={draft.notifications} onChange={(v) => set("notifications", v)} />
        </div>
      )}

      {tab === "updates" && (
        <div className="form">
          <Check label={t("autoUpdate")} checked={draft.autoUpdate} disabled={updatesSupported === false} onChange={(v) => set("autoUpdate", v)} />
          <small className="hint">{t("autoUpdateHint")}</small>
          <UpdateNotice inSettings installBlocked={dirty || saving} onSupportChange={setUpdatesSupported} />
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
                aria-label={t("customProxyUrl")}
                value={customProxy}
                placeholder="http://127.0.0.1:8080 · socks5://127.0.0.1:1080"
                spellCheck={false}
                onChange={(e) => setCustomProxy(e.target.value)}
              />
            )}
          </fieldset>
        </div>
      )}

      {tab === "torrent" && (
        <div className="form">
          <fieldset className="field">
            <span>{t("seedLabel")}</span>
            <div className="radios column">
              {SEED_CHOICES.map((choice) => (
                <label key={choice.value} className="radio">
                  <input
                    type="radio"
                    checked={
                      draft.seedRatio === choice.value ||
                      (choice.value === 100 && !SEED_CHOICES.some((c) => c.value === draft.seedRatio))
                    }
                    onChange={() => set("seedRatio", choice.value)}
                  />
                  {t(choice.label)}
                </label>
              ))}
            </div>
          </fieldset>
          <div className="field-grid">
            <label className="field">
              <span>{t("uploadLimitLabel")}</span>
              <input
                type="number"
                min={0}
                value={Math.round(draft.uploadLimit / 1024)}
                onChange={(e) => set("uploadLimit", number(e.target.value, 0, 10_000_000) * 1024)}
              />
            </label>
            <details className="connection-diagnostics">
              <summary>{t("connectionDiagnostics")}</summary>
            <label className="field">
              <span>{t("torrentPortLabel")}</span>
              <input
                type="number"
                min={0}
                max={65535}
                value={draft.torrentPort}
                onChange={(e) => set("torrentPort", number(e.target.value, 0, 65535))}
              />
            </label>
            <small className="muted">{t("torrentPortHint")}</small>
            </details>
          </div>
          <MagnetHandler onError={onError} />
          <small className="muted">{t("torrentRights")}</small>
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
          <div className="browser-connection-status">
            <p role="status">{t(checkingConnection ? "browserConnectionChecking" : connectionFailed || browserInfo?.connectionError ? "browserConnectionError" : browserConnected ? "browserConnectionConnected" : "browserConnectionWaiting")}</p>
            <button type="button" className="button" disabled={checkingConnection} onClick={() => void retryConnection()}><Icon name="retry" size={14} />{t("browserConnectionRetry")}</button>
          </div>
          <details className="connection-diagnostics">
            <summary>{t("connectionDiagnostics")}</summary>
            {browserInfo?.connectionError && <p className="hint warning">{browserInfo.connectionError}</p>}
            {browserInfo?.bridgeError && <p className="hint warning">{browserInfo.bridgeError}</p>}
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
          </details>
        </div>
      )}
    </Dialog>
  );
}

/** A tiny drawing of each look for the picker. */
function SkinPreview({ skin, dark }: { skin: Skin; dark: boolean }) {
  if (skin === "neon") {
    return (
      <span className="skin-preview neon" aria-hidden>
        <svg viewBox="0 0 100 26" preserveAspectRatio="none">
          <path d="M0 22 L12 18 L24 20 L36 11 L48 14 L60 6 L72 9 L84 4 L100 7 L100 26 L0 26 Z" fill="currentColor" opacity="0.18" />
          <path d="M0 22 L12 18 L24 20 L36 11 L48 14 L60 6 L72 9 L84 4 L100 7" fill="none" stroke="currentColor" strokeWidth="1.5" />
        </svg>
        <i style={{ top: 44 }}>
          <b style={{ width: "72%" }} />
        </i>
        <i style={{ top: 54 }}>
          <b style={{ width: "38%" }} />
        </i>
        <i style={{ top: 64 }}>
          <b style={{ width: "91%" }} />
        </i>
      </span>
    );
  }
  return (
    <span className={`skin-preview minimal${dark ? " dark" : ""}`} aria-hidden>
      <svg viewBox="0 0 160 54" preserveAspectRatio="none" style={{ width: "100%", height: "100%", display: "block" }}>
        <rect width="160" height="10" rx="2" fill={dark ? "#26302c" : "#e3eae5"} />
        {[8, 24, 40, 56, 72].map((x) => <rect key={x} x={x} y="3" width="7" height="4" rx="1" fill="#19b981" />)}
        <rect y="14" width="33" height="40" rx="2" fill={dark ? "#1c2621" : "#edf2ee"} />
        {[21, 30, 39, 48].map((y) => <path key={y} d={`M6 ${y}h20`} stroke={dark ? "#8eaaa0" : "#93a69b"} strokeWidth="2" />)}
        {[18, 28, 38, 48].map((y, row) => (
          <g key={y}>
            <path d={`M41 ${y}h41M91 ${y}h13M114 ${y}h14M139 ${y}h16`} stroke={dark ? "#b1c7bc" : "#768d7e"} strokeWidth="2" />
            <path d={`M91 ${y + 4}h${[8, 5, 13, 3][row]}`} stroke="#19b981" strokeWidth="2" />
          </g>
        ))}
      </svg>
    </span>
  );
}

/** A tiny drawing of each progress window style. */
function StylePreview({ style }: { style: ProgressStyle }) {
  if (style === "graph") {
    return (
      <span className="style-preview graph" aria-hidden>
        <svg viewBox="0 0 100 34" preserveAspectRatio="none">
          <path d="M0 30 L10 22 L20 25 L32 16 L44 19 L56 12 L68 15 L80 9 L90 12 L100 10 L100 34 L0 34 Z" fill="currentColor" opacity="0.2" />
          <path d="M0 30 L10 22 L20 25 L32 16 L44 19 L56 12 L68 15 L80 9 L90 12 L100 10" fill="none" stroke="currentColor" strokeWidth="1.6" />
        </svg>
        <i>
          {[0.9, 0.5, 0.7, 0.4].map((v, i) => (
            <b key={i}>
              <u style={{ width: `${v * 100}%` }} />
            </b>
          ))}
        </i>
      </span>
    );
  }
  if (style === "ring") {
    return (
      <span className="style-preview ring" aria-hidden>
        <svg viewBox="0 0 40 40">
          <circle cx="20" cy="20" r="15" fill="none" strokeWidth="5" className="track" />
          <circle cx="20" cy="20" r="15" fill="none" strokeWidth="5" strokeDasharray="55 100" strokeLinecap="round" transform="rotate(-90 20 20)" stroke="currentColor" />
        </svg>
        <span>
          <em />
          <em />
          <em />
        </span>
      </span>
    );
  }
  return (
    <span className="style-preview classic" aria-hidden>
      {[60, 45, 52].map((w, i) => (
        <span key={i}>
          <em />
          <em style={{ width: `${w}%` }} />
        </span>
      ))}
      <i>
        <b />
      </i>
    </span>
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

/** Lets SDM open magnet links clicked outside the browser (Windows, Linux). */
function MagnetHandler({ onError }: { onError: (message: string) => void }) {
  const { t } = useI18n();
  const [state, setState] = useState<{ supported: boolean; registered: boolean } | null>(null);
  useEffect(() => {
    api.magnetHandler().then(setState).catch(() => setState(null));
  }, []);
  if (!state?.supported) return null;
  return (
    <div className="field">
      <span>{t("magnetHandler")}</span>
      <div className="input-row align-center">
        <span className={state.registered ? "good" : "muted"}>
          {state.registered ? t("magnetHandlerOn") : t("magnetHandlerOff")}
        </span>
        {!state.registered && (
          <button
            type="button"
            className="button"
            onClick={() =>
              api
                .registerMagnetHandler()
                .then(() => api.magnetHandler().then(setState))
                .catch((error) => onError(errorText(error)))
            }
          >
            <Icon name="magnet" size={15} />
            {t("magnetHandlerButton")}
          </button>
        )}
      </div>
    </div>
  );
}
