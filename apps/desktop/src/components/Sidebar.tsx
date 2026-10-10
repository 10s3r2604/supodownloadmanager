import type { Category, Download } from "../api";
import { useI18n } from "../context";
import type { TextKey } from "../i18n";
import { Icon, type IconName, categoryIcon } from "../icons";

export type Filter = "all" | "active" | "completed" | "unfinished" | `cat:${Category}`;

const CATEGORIES: Category[] = ["video", "music", "document", "compressed", "program", "image", "other"];

export function matches(filter: Filter, d: Download): boolean {
  switch (filter) {
    case "all":
      return true;
    case "active":
      return d.state === "downloading" || d.state === "queued";
    case "completed":
      return d.state === "completed";
    case "unfinished":
      return d.state !== "completed";
    default:
      return d.category === filter.slice(4);
  }
}

interface Props {
  filter: Filter;
  downloads: Download[];
  onChange: (filter: Filter) => void;
}

export function Sidebar({ filter, downloads, onChange }: Props) {
  const { t } = useI18n();
  const count = (f: Filter) => downloads.filter((d) => matches(f, d)).length;

  const item = (f: Filter, icon: IconName, label: string) => (
    <li key={f}>
      <button type="button" className={filter === f ? "nav active" : "nav"} onClick={() => onChange(f)}>
        <Icon name={icon} size={16} />
        <span className="nav-label">{label}</span>
        <span className="count">{count(f) || ""}</span>
      </button>
    </li>
  );

  return (
    <nav className="sidebar">
      <ul>
        {item("all", "list", t("all"))}
        {item("active", "download", t("active"))}
        {item("completed", "check", t("completed"))}
        {item("unfinished", "clock", t("unfinished"))}
      </ul>
      <h3>{t("categories")}</h3>
      <ul>
        {CATEGORIES.map((c) => item(`cat:${c}`, categoryIcon[c], t(`cat.${c}` as TextKey)))}
      </ul>
    </nav>
  );
}
