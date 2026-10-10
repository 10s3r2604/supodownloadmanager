//! Finding installed browsers and helping the user install the extension.
//!
//! Browsers do not let other programs install extensions silently. SDM can
//! open the right page in the chosen browser; the user confirms there.
//! Before the extension is published in the stores, SDM copies it to a
//! folder and opens the browser's extension page, where it is loaded with
//! "Load unpacked" (Chromium) or "Load Temporary Add-on" (Firefox).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// Chrome Web Store page of the extension, once it is published. Works for
/// every Chromium based browser.
const CHROME_WEB_STORE_URL: Option<&str> = None;
/// addons.mozilla.org page of the extension, once it is published.
const FIREFOX_ADDONS_URL: Option<&str> = None;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Chromium,
    Firefox,
}

impl Engine {
    /// Folder of the matching extension build.
    pub fn extension_folder(self) -> &'static str {
        match self {
            Engine::Chromium => "chromium",
            Engine::Firefox => "firefox",
        }
    }
}

struct Known {
    id: &'static str,
    name: &'static str,
    engine: Engine,
    /// Paths with `{Variable}` placeholders for environment variables.
    windows: &'static [&'static str],
    macos: &'static [&'static str],
    /// Command names looked up in `PATH`.
    linux: &'static [&'static str],
    extensions_page: &'static str,
}

const KNOWN: &[Known] = &[
    Known {
        id: "chrome",
        name: "Google Chrome",
        engine: Engine::Chromium,
        windows: &[
            r"{ProgramFiles}\Google\Chrome\Application\chrome.exe",
            r"{ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
            r"{LOCALAPPDATA}\Google\Chrome\Application\chrome.exe",
        ],
        macos: &["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"],
        linux: &["google-chrome", "google-chrome-stable"],
        extensions_page: "chrome://extensions",
    },
    Known {
        id: "edge",
        name: "Microsoft Edge",
        engine: Engine::Chromium,
        windows: &[
            r"{ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe",
            r"{ProgramFiles}\Microsoft\Edge\Application\msedge.exe",
        ],
        macos: &["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"],
        linux: &["microsoft-edge", "microsoft-edge-stable"],
        extensions_page: "edge://extensions",
    },
    Known {
        id: "coccoc",
        name: "Cốc Cốc",
        engine: Engine::Chromium,
        windows: &[
            r"{LOCALAPPDATA}\CocCoc\Browser\Application\browser.exe",
            r"{ProgramFiles}\CocCoc\Browser\Application\browser.exe",
            r"{ProgramFiles(x86)}\CocCoc\Browser\Application\browser.exe",
        ],
        macos: &["/Applications/CocCoc.app/Contents/MacOS/CocCoc"],
        linux: &["coccoc-browser", "coccoc"],
        extensions_page: "chrome://extensions",
    },
    Known {
        id: "brave",
        name: "Brave",
        engine: Engine::Chromium,
        windows: &[
            r"{ProgramFiles}\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"{ProgramFiles(x86)}\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"{LOCALAPPDATA}\BraveSoftware\Brave-Browser\Application\brave.exe",
        ],
        macos: &["/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"],
        linux: &["brave-browser", "brave"],
        extensions_page: "brave://extensions",
    },
    Known {
        id: "opera",
        name: "Opera",
        engine: Engine::Chromium,
        windows: &[
            r"{LOCALAPPDATA}\Programs\Opera\opera.exe",
            r"{ProgramFiles}\Opera\opera.exe",
        ],
        macos: &["/Applications/Opera.app/Contents/MacOS/Opera"],
        linux: &["opera"],
        extensions_page: "opera://extensions",
    },
    Known {
        id: "vivaldi",
        name: "Vivaldi",
        engine: Engine::Chromium,
        windows: &[
            r"{LOCALAPPDATA}\Vivaldi\Application\vivaldi.exe",
            r"{ProgramFiles}\Vivaldi\Application\vivaldi.exe",
        ],
        macos: &["/Applications/Vivaldi.app/Contents/MacOS/Vivaldi"],
        linux: &["vivaldi", "vivaldi-stable"],
        extensions_page: "vivaldi://extensions",
    },
    Known {
        id: "chromium",
        name: "Chromium",
        engine: Engine::Chromium,
        windows: &[r"{LOCALAPPDATA}\Chromium\Application\chrome.exe"],
        macos: &["/Applications/Chromium.app/Contents/MacOS/Chromium"],
        linux: &["chromium", "chromium-browser"],
        extensions_page: "chrome://extensions",
    },
    Known {
        id: "firefox",
        name: "Firefox",
        engine: Engine::Firefox,
        windows: &[
            r"{ProgramFiles}\Mozilla Firefox\firefox.exe",
            r"{ProgramFiles(x86)}\Mozilla Firefox\firefox.exe",
        ],
        macos: &["/Applications/Firefox.app/Contents/MacOS/firefox"],
        linux: &["firefox", "firefox-esr"],
        extensions_page: "about:debugging#/runtime/this-firefox",
    },
];

/// An installed browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Browser {
    pub id: String,
    pub name: String,
    pub engine: Engine,
    pub executable: PathBuf,
    pub extensions_page: String,
}

/// How the extension is being installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallInfo {
    /// `store` when the store page was opened, `manual` when the user has to
    /// load the extension folder.
    pub method: &'static str,
    pub browser: String,
    pub engine: Engine,
    pub extension_dir: PathBuf,
    pub extensions_page: String,
}

/// Browsers installed on this computer.
pub fn detect() -> Vec<Browser> {
    detect_with(
        std::env::consts::OS,
        &|name| std::env::var_os(name),
        &|path| path.is_file(),
    )
}

fn detect_with(
    os: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<Browser> {
    KNOWN
        .iter()
        .filter_map(|known| {
            let executable = match os {
                "windows" => known
                    .windows
                    .iter()
                    .filter_map(|pattern| expand(pattern, env))
                    .find(|path| exists(path)),
                "macos" => known
                    .macos
                    .iter()
                    .map(PathBuf::from)
                    .find(|path| exists(path)),
                _ => known
                    .linux
                    .iter()
                    .find_map(|command| find_in_path(command, env, exists)),
            }?;
            Some(Browser {
                id: known.id.to_owned(),
                name: known.name.to_owned(),
                engine: known.engine,
                executable,
                extensions_page: known.extensions_page.to_owned(),
            })
        })
        .collect()
}

/// Replaces `{Variable}` with the value of the environment variable.
fn expand(pattern: &str, env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let start = pattern.find('{')?;
    let end = pattern.find('}')?;
    let value = env(&pattern[start + 1..end])?;
    let mut path = OsString::from(&pattern[..start]);
    path.push(value);
    path.push(&pattern[end + 1..]);
    Some(PathBuf::from(path))
}

fn find_in_path(
    command: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let paths = env("PATH")?;
    // Only used on Linux and other Unix systems, where `:` separates entries.
    paths
        .to_string_lossy()
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(command))
        .find(|path| exists(path))
}

/// Opens the store page, or prepares the extension folder and opens the
/// browser's extension page.
pub fn install(browser: &Browser, bundled: &Path, data_dir: &Path) -> io::Result<InstallInfo> {
    let folder = browser.engine.extension_folder();
    let extension_dir = data_dir.join("extension").join(folder);
    let store = match browser.engine {
        Engine::Chromium => CHROME_WEB_STORE_URL,
        Engine::Firefox => FIREFOX_ADDONS_URL,
    };
    let method = match store {
        Some(url) => {
            open_in(browser, url)?;
            "store"
        }
        None => {
            if extension_dir.exists() {
                std::fs::remove_dir_all(&extension_dir)?;
            }
            copy_dir(&bundled.join(folder), &extension_dir)?;
            open_in(browser, &browser.extensions_page)?;
            "manual"
        }
    };
    Ok(InstallInfo {
        method,
        browser: browser.name.clone(),
        engine: browser.engine,
        extension_dir,
        extensions_page: browser.extensions_page.clone(),
    })
}

/// Starts the browser with a URL. A running browser opens it in a new tab.
fn open_in(browser: &Browser, url: &str) -> io::Result<()> {
    let mut child = Command::new(&browser.executable)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reap the process when the browser exits, so it never lingers.
    std::thread::spawn(move || child.wait());
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| vars.get(name).map(OsString::from)
    }

    #[test]
    fn expands_windows_paths() {
        let env = env(&[("LOCALAPPDATA", r"C:\Users\an\AppData\Local")]);
        assert_eq!(
            expand(r"{LOCALAPPDATA}\CocCoc\browser.exe", &env),
            Some(PathBuf::from(
                r"C:\Users\an\AppData\Local\CocCoc\browser.exe"
            ))
        );
        assert_eq!(expand(r"{ProgramFiles}\x.exe", &env), None);
    }

    #[test]
    fn detects_windows_browsers() {
        let env = env(&[
            ("ProgramFiles", r"C:\Program Files"),
            ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
            ("LOCALAPPDATA", r"C:\Users\an\AppData\Local"),
        ]);
        let installed = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Users\an\AppData\Local\CocCoc\Browser\Application\browser.exe",
            r"C:\Program Files\Mozilla Firefox\firefox.exe",
        ];
        let browsers = detect_with("windows", &env, &|p| {
            installed.contains(&p.to_string_lossy().as_ref())
        });
        let ids: Vec<_> = browsers.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["chrome", "coccoc", "firefox"]);
        assert_eq!(browsers[2].engine, Engine::Firefox);
    }

    #[test]
    fn detects_linux_browsers_in_path() {
        let env = env(&[("PATH", "/usr/local/bin:/usr/bin")]);
        let browsers = detect_with("linux", &env, &|p| {
            p == Path::new("/usr/bin/chromium") || p == Path::new("/usr/bin/firefox")
        });
        let found: Vec<_> = browsers
            .iter()
            .map(|b| (b.id.as_str(), b.executable.clone()))
            .collect();
        assert_eq!(
            found,
            [
                ("chromium", PathBuf::from("/usr/bin/chromium")),
                ("firefox", PathBuf::from("/usr/bin/firefox"))
            ]
        );
    }

    #[test]
    fn detects_macos_apps() {
        let browsers = detect_with("macos", &env(&[]), &|p| {
            p == Path::new("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge")
        });
        assert_eq!(browsers.len(), 1);
        assert_eq!(browsers[0].extensions_page, "edge://extensions");
    }

    #[test]
    fn copies_extension_folders() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src");
        std::fs::create_dir_all(source.join("icons")).unwrap();
        std::fs::write(source.join("manifest.json"), "{}").unwrap();
        std::fs::write(source.join("icons").join("16.png"), "png").unwrap();
        let target = dir.path().join("out");
        copy_dir(&source, &target).unwrap();
        assert!(target.join("manifest.json").is_file());
        assert!(target.join("icons").join("16.png").is_file());
    }
}
