//! Browser cookies handed to yt-dlp, so videos that need a login work.

use serde::{Deserialize, Serialize};

/// One cookie as the browser extension reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// `.example.com` for the domain and its subdomains, `example.com` for
    /// that host only.
    pub domain: String,
    #[serde(default = "root_path")]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub host_only: bool,
    /// Unix time in seconds; missing for session cookies.
    #[serde(default)]
    pub expires: Option<f64>,
}

fn root_path() -> String {
    "/".into()
}

/// The cookies in the Netscape `cookies.txt` format that yt-dlp reads.
pub fn netscape(cookies: &[Cookie]) -> String {
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for c in cookies {
        let fields = [&c.name, &c.value, &c.domain, &c.path];
        if c.name.is_empty()
            || c.domain.is_empty()
            || fields.iter().any(|f| f.contains(['\t', '\n', '\r']))
        {
            continue;
        }
        let host = c.domain.trim_start_matches('.');
        let (domain, subdomains) = if c.host_only {
            (host.to_owned(), "FALSE")
        } else {
            (format!(".{host}"), "TRUE")
        };
        let expires = c.expires.map_or(0, |t| t.max(0.0) as u64);
        let secure = if c.secure { "TRUE" } else { "FALSE" };
        let path = if c.path.is_empty() { "/" } else { &c.path };
        out.push_str(&format!(
            "{domain}\t{subdomains}\t{path}\t{secure}\t{expires}\t{}\t{}\n",
            c.name, c.value
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str, domain: &str, host_only: bool) -> Cookie {
        Cookie {
            name: name.into(),
            value: "v".into(),
            domain: domain.into(),
            path: "/".into(),
            secure: true,
            host_only,
            expires: Some(1_900_000_000.5),
        }
    }

    #[test]
    fn writes_netscape_cookie_files() {
        let mut session = cookie("SID", ".youtube.com", false);
        session.expires = None;
        session.secure = false;
        let text = netscape(&[
            session,
            cookie("PREF", "www.youtube.com", true),
            cookie("bad\tname", ".youtube.com", false),
        ]);
        assert_eq!(
            text,
            "# Netscape HTTP Cookie File\n\
             .youtube.com\tTRUE\t/\tFALSE\t0\tSID\tv\n\
             www.youtube.com\tFALSE\t/\tTRUE\t1900000000\tPREF\tv\n"
        );
    }
}
