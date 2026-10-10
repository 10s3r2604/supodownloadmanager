//! Progress bars and messages.

use std::time::Duration;

use indicatif::{HumanBytes, HumanDuration, MultiProgress, ProgressBar, ProgressStyle};
use sdm_core::{Phase, Progress};

const LABEL_WIDTH: usize = 28;

/// Prints messages above the progress bars, or plainly when there are none.
#[derive(Clone)]
pub struct Ui {
    bars: MultiProgress,
}

impl Ui {
    pub fn new(quiet: bool) -> Self {
        let bars = MultiProgress::new();
        if quiet {
            bars.set_draw_target(indicatif::ProgressDrawTarget::hidden());
        }
        Self { bars }
    }

    pub fn line(&self, message: impl AsRef<str>) {
        if self.bars.is_hidden() {
            println!("{}", message.as_ref());
        } else {
            let _ = self.bars.println(message);
        }
    }

    pub fn add_bar(&self, label: &str) -> ProgressBar {
        let bar = self.bars.add(ProgressBar::new_spinner());
        bar.set_style(spinner_style());
        bar.set_prefix(fit(label, LABEL_WIDTH));
        bar.set_message("waiting");
        bar
    }
}

fn bar_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{prefix:28.bold} [{bar:30.cyan/blue}] {percent:>3}% {bytes:>10} / {total_bytes:<10} {msg}",
    )
    .expect("valid template")
    .progress_chars("=> ")
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{prefix:28.bold} {spinner:.cyan} {bytes:>10} {msg}")
        .expect("valid template")
}

/// Refreshes a bar from a progress snapshot.
pub fn update_bar(bar: &ProgressBar, progress: &Progress) {
    if let Some(name) = &progress.file_name {
        bar.set_prefix(fit(name, LABEL_WIDTH));
    }
    if let Some(total) = progress.total.filter(|&t| t > 0)
        && bar.length() != Some(total)
    {
        bar.set_style(bar_style());
        bar.set_length(total);
    }
    bar.set_position(progress.downloaded);
    bar.set_message(match progress.phase {
        Phase::Connecting => "connecting...".to_owned(),
        Phase::Finishing => "finishing...".to_owned(),
        _ => {
            let mut parts = vec![format!("{}/s", HumanBytes(progress.speed))];
            if let Some(eta) = progress.eta() {
                parts.push(format!("{} left", HumanDuration(eta)));
            }
            if progress.connections > 0 {
                let plural = if progress.connections == 1 { "" } else { "s" };
                parts.push(format!("{} connection{plural}", progress.connections));
            }
            parts.join(" · ")
        }
    });
}

/// A short name for a URL to show before the server tells us the real one.
pub fn label_for_url(url: &str) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    without_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(url)
        .to_owned()
}

/// Shortens text to `width` characters, putting an ellipsis in the middle.
pub fn fit(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_owned();
    }
    let keep = width - 1;
    let head: String = text.chars().take(keep / 2).collect();
    let tail: String = text.chars().skip(count - (keep - keep / 2)).collect();
    format!("{head}…{tail}")
}

/// Describes a finished download, e.g. `11.4 MiB in 3 seconds (3.8 MiB/s)`.
/// Only bytes fetched in this session count towards the speed.
pub fn summary(size: u64, resumed_from: u64, elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64().max(0.001);
    let fetched = size.saturating_sub(resumed_from);
    let mut text = format!(
        "{} in {} ({}/s)",
        HumanBytes(size),
        HumanDuration(elapsed),
        HumanBytes((fetched as f64 / seconds) as u64)
    );
    if resumed_from > 0 && size > 0 {
        let percent = resumed_from as f64 * 100.0 / size as f64;
        text.push_str(&format!(", resumed from {percent:.0}%"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(label_for_url("https://x.org/a/file.zip?x=1"), "file.zip");
        assert_eq!(label_for_url("https://x.org/a/dir/"), "dir");
    }

    #[test]
    fn summaries() {
        let second = Duration::from_secs(1);
        assert_eq!(
            summary(2048, 0, second),
            "2.00 KiB in 1 second (2.00 KiB/s)"
        );
        assert_eq!(
            summary(2048, 1024, second),
            "2.00 KiB in 1 second (1.00 KiB/s), resumed from 50%"
        );
    }

    #[test]
    fn fits_text() {
        assert_eq!(fit("short", 10), "short");
        let fitted = fit("a-very-long-file-name.tar.gz", 10);
        assert_eq!(fitted.chars().count(), 10);
        assert_eq!(fitted, "a-ve…ar.gz");
    }
}
