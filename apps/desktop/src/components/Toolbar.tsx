import { useI18n } from "../context";
import { Icon, type IconName } from "../icons";

interface Props {
  canResume: boolean;
  canPause: boolean;
  canDelete: boolean;
  query: string;
  onQuery: (query: string) => void;
  onAdd: () => void;
  onResume: () => void;
  onPause: () => void;
  onDelete: () => void;
  onResumeAll: () => void;
  onPauseAll: () => void;
  onBrowsers: () => void;
  onSettings: () => void;
}

export function Toolbar(props: Props) {
  const { t } = useI18n();
  const button = (icon: IconName, label: string, onClick: () => void, enabled = true, primary = false) => (
    <button
      type="button"
      className={primary ? "tool primary" : "tool"}
      onClick={onClick}
      disabled={!enabled}
      title={label}
    >
      <Icon name={icon} />
      <span>{label}</span>
    </button>
  );

  return (
    <header className="toolbar">
      {button("plus", t("add"), props.onAdd, true, true)}
      <span className="divider" />
      {button("play", t("resume"), props.onResume, props.canResume)}
      {button("pause", t("pause"), props.onPause, props.canPause)}
      {button("trash", t("delete"), props.onDelete, props.canDelete)}
      <span className="divider" />
      {button("playAll", t("resumeAll"), props.onResumeAll)}
      {button("pauseAll", t("pauseAll"), props.onPauseAll)}
      <span className="spacer" />
      <label className="search">
        <Icon name="search" size={16} />
        <input
          type="search"
          value={props.query}
          placeholder={t("search")}
          aria-label={t("search")}
          onChange={(event) => props.onQuery(event.target.value)}
        />
      </label>
      {button("globe", t("browsers"), props.onBrowsers)}
      {button("sliders", t("settings"), props.onSettings)}
    </header>
  );
}
