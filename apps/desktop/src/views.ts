// Data and actions for the download list.

import type { AppInfo, Download, PowerAction, Queue, Settings } from "./api";
import type { Filter, History, Totals, TypeFilter } from "./model";

export interface RowActions {
  select: (id: number, mode: "single" | "toggle" | "range") => void;
  selectAll: (ids: number[]) => void;
  details: (download: Download) => void;
  /** The obvious thing to do: pause, resume, retry or open the file. */
  primary: (download: Download) => void;
  open: (download: Download) => void;
  showInFolder: (download: Download) => void;
  pause: (ids: number[]) => void;
  resume: (ids: number[]) => void;
  copyLink: (download: Download) => void;
  remove: (ids: number[]) => void;
  openProgress: (download: Download) => void;
  moveToQueue: (ids: number[], queue: string) => void;
  /** Asks for a new link for an unfinished download. */
  newAddress: (download: Download) => void;
  menu: (event: { clientX: number; clientY: number }, download: Download) => void;
}

export interface ViewProps {
  downloads: Download[];
  visible: Download[];
  filter: Filter;
  setFilter: (filter: Filter) => void;
  typeFilter: TypeFilter;
  setTypeFilter: (filter: TypeFilter) => void;
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
  queues: Queue[];
  /** A queue id, or "all". */
  queueFilter: string;
  setQueueFilter: (queue: string) => void;
  afterAll: PowerAction;
  setAfterAll: (action: PowerAction) => void;
  onScheduler: () => void;
  onStartQueue: (queue: string) => void;
  onStopQueue: (queue: string) => void;
}
