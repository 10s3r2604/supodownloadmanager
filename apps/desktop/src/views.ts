// What a look (skin) receives from the app.

import type { AppInfo, Download, Settings } from "./api";
import type { Filter, History, Totals } from "./model";

export interface RowActions {
  select: (id: number, mode: "single" | "toggle" | "range") => void;
  /** The obvious thing to do: pause, resume, retry or open the file. */
  primary: (download: Download) => void;
  open: (download: Download) => void;
  showInFolder: (download: Download) => void;
  pause: (ids: number[]) => void;
  resume: (ids: number[]) => void;
  copyLink: (download: Download) => void;
  remove: (ids: number[]) => void;
  openProgress: (download: Download) => void;
  menu: (event: { clientX: number; clientY: number }, download: Download) => void;
}

export interface ViewProps {
  downloads: Download[];
  visible: Download[];
  filter: Filter;
  setFilter: (filter: Filter) => void;
  query: string;
  setQuery: (query: string) => void;
  selected: number[];
  actions: RowActions;
  settings: Settings;
  info: AppInfo | null;
  history: History;
  totals: Totals;
  onAdd: () => void;
  onResumeAll: () => void;
  onPauseAll: () => void;
  onSettings: () => void;
  onBrowsers: () => void;
}
