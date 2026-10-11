// Queues: how many files each downloads at once, when it starts and stops by
// itself, and what happens when it is done.

import { useEffect, useState } from "react";

import { type Download, MAIN_QUEUE, type PowerAction, type Queue, type Settings, api } from "../api";
import { errorText, useI18n } from "../context";
import type { TextKey, Translate } from "../i18n";
import { Icon } from "../icons";
import { Dialog } from "./Dialog";

interface Props {
  queues: Queue[];
  downloads: Download[];
  settings: Settings;
  onClose: () => void;
  onError: (message: string) => void;
  /** Called after a change, so the list can show it. */
  onChanged: () => void;
  onSettings: (settings: Settings) => void;
}

export const POWER_ACTIONS: PowerAction[] = ["nothing", "quit", "sleep", "hibernate", "shutdown"];
const DAYS = [0, 1, 2, 3, 4, 5, 6];

type Repeat = "once" | "daily" | "days";

const blank = (): Queue => ({
  id: "",
  name: "",
  maxParallel: 2,
  schedule: { enabled: true, start: "01:00", stop: null, days: [...DAYS] },
  whenDone: "nothing",
});

export function queueName(queue: Queue | undefined, t: Translate) {
  return !queue || queue.id === MAIN_QUEUE ? t("mainQueue") : queue.name;
}

/** "01:00–06:30 · Every day", for the list of queues. */
export function scheduleSummary(queue: Queue, t: Translate) {
  const s = queue.schedule;
  if (!s.enabled || (!s.start && !s.stop)) return t("scheduleOff");
  const times = [s.start ?? "", s.stop ?? ""].filter(Boolean).join("–");
  const days =
    s.days.length === 0
      ? t("repeatOnce")
      : s.days.length === 7
        ? t("repeatDaily")
        : s.days.map((d) => t(`day.${d}` as TextKey)).join(" ");
  return `${times} · ${days}`;
}

export function SchedulerDialog({ queues, downloads, settings, onClose, onError, onChanged, onSettings }: Props) {
  const { t } = useI18n();
  const [chosen, setChosen] = useState(queues[0]?.id ?? MAIN_QUEUE);
  const [draft, setDraft] = useState<Queue>(() => queues[0] ?? blank());
  const [busy, setBusy] = useState(false);
  const isNew = draft.id === "";
  const isMain = draft.id === MAIN_QUEUE;
  const parallel = isMain ? settings.maxParallel : draft.maxParallel;
  const [mainParallel, setMainParallel] = useState(settings.maxParallel);

  // Show the chosen queue as it is saved.
  useEffect(() => {
    if (chosen === "") return;
    const queue = queues.find((q) => q.id === chosen);
    if (queue) setDraft(structuredClone(queue));
  }, [chosen, queues]);

  const unfinished = (id: string) => downloads.filter((d) => d.queue === id && d.state !== "completed").length;
  const schedule = draft.schedule;
  const repeat: Repeat = schedule.days.length === 0 ? "once" : schedule.days.length === 7 ? "daily" : "days";
  const setSchedule = (change: Partial<Queue["schedule"]>) =>
    setDraft((q) => ({ ...q, schedule: { ...q.schedule, ...change } }));

  async function act(action: () => Promise<unknown>) {
    setBusy(true);
    try {
      await action();
      onChanged();
    } catch (error) {
      onError(errorText(error));
    } finally {
      setBusy(false);
    }
  }

  const save = () =>
    act(async () => {
      if (isMain && mainParallel !== settings.maxParallel) {
        onSettings(await api.saveSettings({ ...settings, maxParallel: mainParallel }));
      }
      const saved = await api.saveQueue(draft);
      setDraft(saved);
      setChosen(saved.id);
    });

  return (
    <Dialog
      title={t("schedulerTitle")}
      onClose={onClose}
      wide
      footer={
        <>
          {!isMain && !isNew && (
            <button
              type="button"
              className="button danger-text sched-delete"
              title={t("deleteQueueHint")}
              disabled={busy}
              onClick={() =>
                act(async () => {
                  await api.deleteQueue(draft.id);
                  setChosen(MAIN_QUEUE);
                })
              }
            >
              <Icon name="trash" size={15} /> {t("deleteQueue")}
            </button>
          )}
          {!isNew && (
            <>
              <button type="button" className="button" disabled={busy} onClick={() => act(() => api.stopQueue(draft.id))}>
                <Icon name="pause" size={15} /> {t("stopQueue")}
              </button>
              <button type="button" className="button" disabled={busy} onClick={() => act(() => api.startQueue(draft.id))}>
                <Icon name="play" size={15} /> {t("startQueue")}
              </button>
            </>
          )}
          <button type="button" className="button primary" disabled={busy} onClick={save}>
            {t("save")}
          </button>
        </>
      }
    >
      <div className="sched">
        <nav className="sched-list" aria-label={t("queue")}>
          {queues.map((q) => (
            <button
              type="button"
              key={q.id}
              className={q.id === chosen ? "on" : ""}
              aria-current={q.id === chosen ? "true" : undefined}
              onClick={() => setChosen(q.id)}
            >
              <strong>{queueName(q, t)}</strong>
              <small>{scheduleSummary(q, t)}</small>
              <small>{t("queueFiles", { n: unfinished(q.id) })}</small>
            </button>
          ))}
          {isNew && (
            <button type="button" className="on" aria-current="true">
              <strong>{draft.name || t("newQueue")}</strong>
              <small>{scheduleSummary(draft, t)}</small>
            </button>
          )}
          <button
            type="button"
            className="sched-new"
            disabled={isNew}
            onClick={() => {
              setChosen("");
              setDraft({ ...blank(), name: t("newQueue") });
            }}
          >
            <Icon name="plus" size={15} /> {t("newQueue")}
          </button>
        </nav>

        <div className="form sched-form">
          {!isMain && (
            <label className="field">
              <span>{t("queueName")}</span>
              <input
                autoFocus={isNew}
                value={draft.name}
                maxLength={60}
                onChange={(e) => setDraft((q) => ({ ...q, name: e.target.value }))}
              />
            </label>
          )}
          <label className="field sched-parallel">
            <span>{t("maxParallel")}</span>
            <input
              type="number"
              min={1}
              max={16}
              value={isMain ? mainParallel : parallel}
              onChange={(e) => {
                const value = Math.min(16, Math.max(1, Math.round(Number(e.target.value)) || 1));
                if (isMain) setMainParallel(value);
                else setDraft((q) => ({ ...q, maxParallel: value }));
              }}
            />
          </label>
          {!isMain && <p className="hint muted sched-hint">{t("queueLaterHint")}</p>}

          <label className="check">
            <input
              type="checkbox"
              checked={schedule.enabled}
              onChange={(e) =>
                setSchedule(
                  // A schedule needs a time to do anything.
                  e.target.checked && !schedule.start && !schedule.stop
                    ? { enabled: true, start: "01:00" }
                    : { enabled: e.target.checked },
                )
              }
            />
            {t("scheduleOn")}
          </label>

          <fieldset className={`sched-times${schedule.enabled ? "" : " off"}`} disabled={!schedule.enabled}>
            <div className="field-grid">
              <label className="field">
                <span>
                  <input
                    type="checkbox"
                    checked={schedule.start != null}
                    aria-label={t("startAt")}
                    onChange={(e) => setSchedule({ start: e.target.checked ? "01:00" : null })}
                  />{" "}
                  {t("startAt")}
                </span>
                <input
                  type="time"
                  value={schedule.start ?? ""}
                  disabled={schedule.start == null}
                  onChange={(e) => setSchedule({ start: e.target.value || null })}
                />
              </label>
              <label className="field">
                <span>
                  <input
                    type="checkbox"
                    checked={schedule.stop != null}
                    aria-label={t("stopAt")}
                    onChange={(e) => setSchedule({ stop: e.target.checked ? "06:00" : null })}
                  />{" "}
                  {t("stopAt")}
                </span>
                <input
                  type="time"
                  value={schedule.stop ?? ""}
                  disabled={schedule.stop == null}
                  onChange={(e) => setSchedule({ stop: e.target.value || null })}
                />
              </label>
            </div>

            <div className="field">
              <span>{t("repeat")}</span>
              <div className="radios">
                {(["once", "daily", "days"] as const).map((mode) => (
                  <label key={mode} className="radio">
                    <input
                      type="radio"
                      name="repeat"
                      checked={repeat === mode}
                      onChange={() =>
                        setSchedule({ days: mode === "once" ? [] : mode === "daily" ? [...DAYS] : [0, 1, 2, 3, 4] })
                      }
                    />
                    {t(mode === "once" ? "repeatOnce" : mode === "daily" ? "repeatDaily" : "repeatDays")}
                  </label>
                ))}
              </div>
              {repeat === "days" && (
                <div className="sched-days" role="group" aria-label={t("repeatDays")}>
                  {DAYS.map((day) => {
                    const on = schedule.days.includes(day);
                    return (
                      <button
                        type="button"
                        key={day}
                        aria-pressed={on}
                        className={on ? "on" : ""}
                        onClick={() => {
                          const days = on ? schedule.days.filter((d) => d !== day) : [...schedule.days, day].sort();
                          // Keep at least one day; none would mean "once".
                          if (days.length > 0) setSchedule({ days });
                        }}
                      >
                        {t(`day.${day}` as TextKey)}
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          </fieldset>

          <label className="field">
            <span>{t("whenQueueDone")}</span>
            <select value={draft.whenDone} onChange={(e) => setDraft((q) => ({ ...q, whenDone: e.target.value as PowerAction }))}>
              {POWER_ACTIONS.map((action) => (
                <option key={action} value={action}>
                  {t(`power.${action}` as TextKey)}
                </option>
              ))}
            </select>
          </label>
        </div>
      </div>
    </Dialog>
  );
}
