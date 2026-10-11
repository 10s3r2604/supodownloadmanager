//! Batch downloads: one address with numbered parts, such as
//! `https://example.com/photos/img[001-120].jpg`, becomes many links. And
//! the links of a web page, sent by the browser extension.

use crate::bridge::PageLinks;
use crate::manager::BatchLink;

/// The most links one pattern may produce.
pub const MAX_LINKS: usize = 10_000;

/// The links of a page, ready for the "add links" dialog.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinksPrompt {
    pub page_url: String,
    pub title: Option<String>,
    pub links: Vec<BatchLink>,
}

/// Turns the links the browser found on a page into downloads to pick
/// from: each without duplicates, with the page as referer, and with the
/// page's cookies only for links on the page's own site.
pub fn from_page(page: PageLinks) -> LinksPrompt {
    let host = |url: &str| url::Url::parse(url).ok()?.host_str().map(str::to_owned);
    let page_host = host(&page.page_url);
    let mut seen = std::collections::HashSet::new();
    let links = page
        .links
        .into_iter()
        .filter_map(|link| {
            let url = link
                .url
                .split('#')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            if url.is_empty() || !seen.insert(url.clone()) {
                return None;
            }
            let mut headers = vec![("Referer".to_owned(), page.page_url.clone())];
            if let Some(agent) = page.user_agent.as_deref().filter(|a| !a.is_empty()) {
                headers.push(("User-Agent".into(), agent.to_owned()));
            }
            if let Some(cookies) = page.cookies.as_deref().filter(|c| !c.is_empty())
                && page_host.is_some()
                && host(&url) == page_host
            {
                headers.push(("Cookie".into(), cookies.to_owned()));
            }
            let text = link
                .text
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|t| !t.is_empty())
                .map(|t| t.chars().take(200).collect());
            Some(BatchLink {
                url,
                file_name: None,
                headers,
                text,
            })
        })
        .collect();
    LinksPrompt {
        page_url: page.page_url,
        title: page.title.filter(|t| !t.trim().is_empty()),
        links,
    }
}

/// A `[from-to]` or `[from-to:step]` part of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Range {
    /// Numbers, padded with zeros to `width` digits.
    Numbers {
        from: u64,
        to: u64,
        step: u64,
        width: usize,
    },
    /// Letters `from..=to`.
    Letters { from: char, to: char, step: u64 },
}

impl Range {
    fn values(&self) -> Vec<String> {
        match *self {
            Range::Numbers {
                from,
                to,
                step,
                width,
            } => {
                let step = usize::try_from(step).unwrap_or(usize::MAX);
                if from <= to {
                    (from..=to)
                        .step_by(step)
                        .map(|n| format!("{n:0width$}"))
                        .collect()
                } else {
                    (to..=from)
                        .rev()
                        .step_by(step)
                        .map(|n| format!("{n:0width$}"))
                        .collect()
                }
            }
            Range::Letters { from, to, step } => {
                let step = usize::try_from(step).unwrap_or(usize::MAX);
                let letters: Vec<char> = if from <= to {
                    (from..=to).collect()
                } else {
                    (to..=from).rev().collect()
                };
                letters
                    .into_iter()
                    .step_by(step)
                    .map(String::from)
                    .collect()
            }
        }
    }

    fn len(&self) -> u64 {
        match *self {
            Range::Numbers { from, to, step, .. } => from.abs_diff(to) / step + 1,
            Range::Letters { from, to, step } => {
                u64::from(u32::from(from).abs_diff(u32::from(to))) / step + 1
            }
        }
    }
}

/// Reads the inside of `[...]`, or `None` when it is not a range (so the
/// brackets stay as they are).
fn parse_range(text: &str) -> Option<Result<Range, String>> {
    let (span, step) = match text.split_once(':') {
        Some((span, step)) => (span, Some(step)),
        None => (text, None),
    };
    let (from, to) = span.split_once('-')?;
    let (from, to) = (from.trim(), to.trim());
    let step = match step.map(str::trim) {
        None => 1,
        Some(step) => match step.parse::<u64>() {
            Ok(step) if step > 0 => step,
            _ => return Some(Err(format!("“{text}”: the step must be a number above 0"))),
        },
    };
    let digits = |s: &str| !s.is_empty() && s.len() <= 18 && s.bytes().all(|b| b.is_ascii_digit());
    if digits(from) && digits(to) {
        // A leading zero ("007") asks for padding; a plain "0" does not.
        let padded = |s: &str| s.len() > 1 && s.starts_with('0');
        let width = if padded(from) || padded(to) {
            from.len().max(to.len())
        } else {
            0
        };
        return Some(Ok(Range::Numbers {
            from: from.parse().ok()?,
            to: to.parse().ok()?,
            step,
            width,
        }));
    }
    let letter = |s: &str| {
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphabetic() => Some(c),
            _ => None,
        }
    };
    match (letter(from), letter(to)) {
        (Some(a), Some(b)) if a.is_ascii_lowercase() == b.is_ascii_lowercase() => {
            Some(Ok(Range::Letters {
                from: a,
                to: b,
                step,
            }))
        }
        (Some(_), Some(_)) => Some(Err(format!(
            "“{text}”: use both lowercase or both uppercase letters"
        ))),
        _ => None,
    }
}

enum Part {
    Text(String),
    Range(Range),
}

/// All links a pattern stands for, in order. A pattern without ranges is
/// one link.
pub fn expand(pattern: &str) -> Result<Vec<String>, String> {
    let pattern = pattern.trim();
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|i| open + i) else {
            break;
        };
        match parse_range(&rest[open + 1..close]) {
            Some(range) => {
                text.push_str(&rest[..open]);
                parts.push(Part::Text(std::mem::take(&mut text)));
                parts.push(Part::Range(range?));
            }
            None => text.push_str(&rest[..=close]),
        }
        rest = &rest[close + 1..];
    }
    text.push_str(rest);
    parts.push(Part::Text(text));

    let total = parts.iter().try_fold(1u64, |total, part| match part {
        Part::Range(range) => total.checked_mul(range.len()),
        Part::Text(_) => Some(total),
    });
    if total.is_none_or(|n| n > MAX_LINKS as u64) {
        return Err(format!(
            "this pattern makes more than {MAX_LINKS} links; use a smaller range"
        ));
    }

    let mut links = vec![String::new()];
    for part in parts {
        match part {
            Part::Text(text) => links.iter_mut().for_each(|l| l.push_str(&text)),
            Part::Range(range) => {
                let values = range.values();
                links = links
                    .iter()
                    .flat_map(|link| values.iter().map(move |v| format!("{link}{v}")))
                    .collect();
            }
        }
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::PageLink;

    #[test]
    fn keeps_cookies_on_the_page_site() {
        let link = |url: &str, text: Option<&str>| PageLink {
            url: url.into(),
            text: text.map(Into::into),
        };
        let prompt = from_page(PageLinks {
            page_url: "https://files.example.com/list".into(),
            title: Some(" ".into()),
            links: vec![
                link("https://files.example.com/a.zip#top", Some("  Get\n  A ")),
                link("https://files.example.com/a.zip", None),
                link("https://cdn.example.net/b.mp4", None),
            ],
            cookies: Some("sid=1".into()),
            user_agent: Some("Browser/1".into()),
        });
        assert_eq!(prompt.title, None);
        assert_eq!(prompt.links.len(), 2);
        let a = &prompt.links[0];
        assert_eq!(a.url, "https://files.example.com/a.zip");
        assert_eq!(a.text.as_deref(), Some("Get A"));
        assert!(a.headers.contains(&("Cookie".into(), "sid=1".into())));
        assert!(
            a.headers
                .contains(&("Referer".into(), "https://files.example.com/list".into()))
        );
        let b = &prompt.links[1];
        assert!(!b.headers.iter().any(|(name, _)| name == "Cookie"));
        assert!(
            b.headers
                .contains(&("User-Agent".into(), "Browser/1".into()))
        );
    }

    #[test]
    fn expands_numbers_letters_and_steps() {
        assert_eq!(
            expand("https://x.org/img[1-3].jpg").unwrap(),
            [
                "https://x.org/img1.jpg",
                "https://x.org/img2.jpg",
                "https://x.org/img3.jpg"
            ]
        );
        assert_eq!(
            expand("https://x.org/[008-011].png").unwrap(),
            [
                "https://x.org/008.png",
                "https://x.org/009.png",
                "https://x.org/010.png",
                "https://x.org/011.png"
            ]
        );
        assert_eq!(
            expand("https://x.org/p[0-20:10]").unwrap(),
            ["https://x.org/p0", "https://x.org/p10", "https://x.org/p20"]
        );
        assert_eq!(
            expand("https://x.org/[3-1]").unwrap(),
            ["https://x.org/3", "https://x.org/2", "https://x.org/1"]
        );
        assert_eq!(
            expand("https://x.org/[a-c][1-2]").unwrap(),
            [
                "https://x.org/a1",
                "https://x.org/a2",
                "https://x.org/b1",
                "https://x.org/b2",
                "https://x.org/c1",
                "https://x.org/c2"
            ]
        );
        assert_eq!(expand("https://x.org/[X-Z]").unwrap().len(), 3);
    }

    #[test]
    fn leaves_other_brackets_alone() {
        assert_eq!(
            expand(" https://x.org/a[b]c[1-2]?q=[x] ").unwrap(),
            ["https://x.org/a[b]c1?q=[x]", "https://x.org/a[b]c2?q=[x]"]
        );
        assert_eq!(expand("https://x.org/[1-").unwrap(), ["https://x.org/[1-"]);
        assert_eq!(
            expand("https://x.org/f.zip").unwrap(),
            ["https://x.org/f.zip"]
        );
    }

    #[test]
    fn refuses_bad_and_huge_patterns() {
        assert!(expand("https://x.org/[1-5:0]").is_err());
        assert!(expand("https://x.org/[a-Z]").is_err());
        assert!(expand("https://x.org/[1-100000]").is_err());
        assert!(expand("https://x.org/[1-200][1-200]").is_err());
        assert!(expand("https://x.org/[0-999999999999999999]").is_err());
    }
}
