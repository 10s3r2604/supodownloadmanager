//! Download queues. Each queue runs a few downloads at a time, can start
//! and stop at set times, and can quit the app or turn the computer off
//! when its downloads are done.

use chrono::{Datelike, Duration, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

/// The queue every download is in unless the user picks another one.
pub const MAIN_QUEUE: &str = "main";

/// The longest gap between two schedule checks that still counts (the
/// computer was asleep, say). Longer gaps are skipped.
const MAX_GAP_HOURS: i64 = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub id: String,
    /// Empty for the main queue, whose name the interface translates.
    #[serde(default)]
    pub name: String,
    /// Downloads running at the same time. The main queue uses the
    /// "downloads at the same time" setting instead.
    #[serde(default = "default_parallel")]
    pub max_parallel: u16,
    #[serde(default)]
    pub schedule: Schedule,
    /// What to do once the queue has nothing left to download.
    #[serde(default)]
    pub when_done: PowerAction,
}

fn default_parallel() -> u16 {
    3
}

/// When a queue starts and stops by itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Schedule {
    pub enabled: bool,
    /// `HH:MM`, local time.
    pub start: Option<String>,
    /// `HH:MM`, local time.
    pub stop: Option<String>,
    /// Days it runs on, `0` for Monday to `6` for Sunday. Empty for once:
    /// the schedule turns itself off after it ran.
    pub days: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerAction {
    #[default]
    Nothing,
    /// Quit SDM.
    Quit,
    Sleep,
    Hibernate,
    Shutdown,
}

/// What a schedule asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleEvent {
    Start,
    Stop,
}

impl Queue {
    pub fn main() -> Self {
        Self {
            id: MAIN_QUEUE.into(),
            name: String::new(),
            max_parallel: default_parallel(),
            schedule: Schedule::default(),
            when_done: PowerAction::Nothing,
        }
    }

    pub fn is_main(&self) -> bool {
        self.id == MAIN_QUEUE
    }

    /// Brings every value into its valid range.
    pub fn sanitized(mut self) -> Self {
        self.name = self.name.trim().chars().take(60).collect();
        if self.is_main() {
            self.name.clear();
        } else if self.name.is_empty() {
            self.name = "Queue".into();
        }
        self.max_parallel = self.max_parallel.clamp(1, 16);
        let schedule = &mut self.schedule;
        schedule.start = schedule.start.as_deref().and_then(clock_time);
        schedule.stop = schedule.stop.as_deref().and_then(clock_time);
        schedule.days.retain(|d| *d < 7);
        schedule.days.sort_unstable();
        schedule.days.dedup();
        if schedule.start.is_none() && schedule.stop.is_none() {
            schedule.enabled = false;
        }
        self
    }
}

/// Makes sure a list of queues starts with the main queue and has no
/// duplicates.
pub fn sanitize_all(queues: Vec<Queue>) -> Vec<Queue> {
    let mut out: Vec<Queue> = Vec::with_capacity(queues.len() + 1);
    for queue in queues {
        if queue.id.is_empty() || out.iter().any(|q| q.id == queue.id) {
            continue;
        }
        out.push(queue.sanitized());
    }
    match out.iter().position(Queue::is_main) {
        Some(0) => {}
        Some(index) => {
            let main = out.remove(index);
            out.insert(0, main);
        }
        None => out.insert(0, Queue::main()),
    }
    out
}

/// `HH:MM` in canonical form, or `None` when it is not a time.
fn clock_time(text: &str) -> Option<String> {
    let time = NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()?;
    Some(time.format("%H:%M").to_string())
}

impl Schedule {
    /// The starts and stops due after `from` up to and including `to`, in
    /// order.
    pub fn due(&self, from: NaiveDateTime, to: NaiveDateTime) -> Vec<ScheduleEvent> {
        if !self.enabled || to <= from || to - from > Duration::hours(MAX_GAP_HOURS) {
            return Vec::new();
        }
        let times = [
            (self.start.as_deref(), ScheduleEvent::Start),
            (self.stop.as_deref(), ScheduleEvent::Stop),
        ];
        let mut due = Vec::new();
        let mut day = from.date();
        while day <= to.date() {
            let weekday = u8::try_from(day.weekday().num_days_from_monday()).unwrap_or(0);
            if self.days.is_empty() || self.days.contains(&weekday) {
                for (time, event) in times {
                    let Some(time) = time.and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok())
                    else {
                        continue;
                    };
                    let at = day.and_time(time);
                    if from < at && at <= to {
                        due.push((at, event));
                    }
                }
            }
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
        due.sort_by_key(|(at, _)| *at);
        due.into_iter().map(|(_, event)| event).collect()
    }

    /// Whether a one-time schedule is over after `event`.
    pub fn finished_by(&self, event: ScheduleEvent) -> bool {
        self.days.is_empty()
            && match event {
                ScheduleEvent::Start => self.stop.is_none(),
                ScheduleEvent::Stop => true,
            }
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        // 2026-10-12 is a Monday.
        NaiveDate::from_ymd_opt(2026, 10, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    fn schedule(start: Option<&str>, stop: Option<&str>, days: &[u8]) -> Schedule {
        Schedule {
            enabled: true,
            start: start.map(Into::into),
            stop: stop.map(Into::into),
            days: days.to_vec(),
        }
    }

    #[test]
    fn finds_starts_and_stops() {
        use ScheduleEvent::{Start, Stop};
        let night = schedule(Some("23:30"), Some("06:00"), &[]);
        assert_eq!(night.due(at(12, 23, 29), at(12, 23, 30)), vec![Start]);
        assert_eq!(night.due(at(12, 23, 30), at(12, 23, 31)), vec![]);
        assert_eq!(night.due(at(13, 5, 59), at(13, 6, 0)), vec![Stop]);
        // Asleep through both: they happen in order.
        assert_eq!(night.due(at(12, 23, 0), at(13, 7, 0)), vec![Start, Stop]);
        // Too long ago to count.
        assert_eq!(night.due(at(10, 0, 0), at(13, 7, 0)), vec![]);

        let weekdays = schedule(Some("02:00"), None, &[0, 1, 2, 3, 4]);
        assert_eq!(weekdays.due(at(16, 1, 59), at(16, 2, 0)), vec![Start]); // Friday
        assert_eq!(weekdays.due(at(17, 1, 59), at(17, 2, 0)), vec![]); // Saturday

        let off = Schedule {
            enabled: false,
            ..night
        };
        assert_eq!(off.due(at(12, 23, 29), at(12, 23, 30)), vec![]);
    }

    #[test]
    fn one_time_schedules_end() {
        let once = schedule(Some("01:00"), None, &[]);
        assert!(once.finished_by(ScheduleEvent::Start));
        let window = schedule(Some("01:00"), Some("05:00"), &[]);
        assert!(!window.finished_by(ScheduleEvent::Start));
        assert!(window.finished_by(ScheduleEvent::Stop));
        let daily = schedule(Some("01:00"), None, &[0, 1, 2, 3, 4, 5, 6]);
        assert!(!daily.finished_by(ScheduleEvent::Start));
    }

    #[test]
    fn cleans_up_queues() {
        let queue = Queue {
            id: "q1".into(),
            name: "  ".into(),
            max_parallel: 99,
            schedule: Schedule {
                enabled: true,
                start: Some("7:05".into()),
                stop: Some("25:00".into()),
                days: vec![6, 9, 0, 6],
            },
            when_done: PowerAction::Shutdown,
        }
        .sanitized();
        assert_eq!(queue.name, "Queue");
        assert_eq!(queue.max_parallel, 16);
        assert_eq!(queue.schedule.start.as_deref(), Some("07:05"));
        assert_eq!(queue.schedule.stop, None);
        assert_eq!(queue.schedule.days, vec![0, 6]);
        assert!(queue.schedule.enabled);

        let no_times = Queue {
            schedule: Schedule {
                enabled: true,
                ..Schedule::default()
            },
            ..queue.clone()
        }
        .sanitized();
        assert!(!no_times.schedule.enabled);

        let all = sanitize_all(vec![queue.clone(), queue.clone(), Queue::main()]);
        assert_eq!(
            all.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(),
            ["main", "q1"]
        );
        assert_eq!(sanitize_all(Vec::new()), vec![Queue::main()]);
    }

    #[test]
    fn reads_old_and_partial_json() {
        let queue: Queue = serde_json::from_str(r#"{"id":"q2","name":"Night"}"#).unwrap();
        assert_eq!(queue.max_parallel, 3);
        assert_eq!(queue.when_done, PowerAction::Nothing);
        let action: PowerAction = serde_json::from_str(r#""shutdown""#).unwrap();
        assert_eq!(action, PowerAction::Shutdown);
    }
}
