//! A once-a-day check for a newer release on GitHub.
//!
//! It asks for `releases/latest` and reads the tag from the redirect, using the
//! `curl` that ships with macOS, Windows 10+ and Linux distros, so Drift carries
//! no HTTP or TLS code and holds no memory for it once the check is done. The
//! result is cached in the state folder, so the check runs at most once a day
//! however many windows or launches there are.
//!
//! The daily check can be turned off in Settings. `DRIFT_UPDATE_CHECK=0|1`
//! overrides that setting; debug builds don't check unless it is `1`.
//! "Check for Updates" (status bar, Settings, command palette) always checks.

use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const LATEST: &str = "https://github.com/rk-kabilan/drift/releases/latest";
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const CACHE: &str = "update-check";

/// A release newer than the running version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    /// The release page, where the downloads are.
    pub url: String,
}

/// Whether the daily check is turned on.
pub fn enabled(cx: &gpui::App) -> bool {
    match std::env::var("DRIFT_UPDATE_CHECK").as_deref() {
        Ok("0") => false,
        Ok("1") => true,
        _ => !cfg!(debug_assertions) && cx.global::<crate::settings::Settings>().check_updates,
    }
}

/// The newest release known from the last check, if it is newer than this build.
pub fn cached() -> Option<Update> {
    let (_, version, url) = read_cache()?;
    is_newer(&version, env!("CARGO_PKG_VERSION")).then_some(Update { version, url })
}

/// Whether a day has passed since the last check.
pub fn due() -> bool {
    read_cache().is_none_or(|(at, _, _)| now().saturating_sub(at) >= INTERVAL.as_secs())
}

/// Asks GitHub for the latest release and caches the answer. Blocking: run it
/// off the main thread. `Ok(None)` means this build is the latest; `Err` means
/// the releases page couldn't be read (offline, or nothing released yet).
pub fn check() -> Result<Option<Update>, ()> {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "--max-time",
        "15",
        "-o",
        null,
        "-w",
        "%{redirect_url}",
        LATEST,
    ])
    .stdin(Stdio::null())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let url = cmd
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    match version_from_url(&url) {
        Some(version) => {
            write_cache(&version, &url);
            Ok(cached())
        }
        None => {
            // Offline, or nothing released yet: note the attempt so it isn't
            // retried until tomorrow, and keep what the last good check found.
            let (_, version, url) = read_cache().unwrap_or_default();
            write_cache(&version, &url);
            Err(())
        }
    }
}

/// The update the app knows about, shared by every window.
#[derive(Default)]
pub struct Available(pub Option<Update>);

impl gpui::Global for Available {}

/// Shows the last known update straight away, then checks once a day for as
/// long as the app runs (while the check is turned on).
pub fn start(cx: &mut gpui::App) {
    cx.set_global(Available(cached()));
    cx.spawn(async move |cx| loop {
        let Ok(on) = cx.update(|cx| enabled(cx)) else {
            break;
        };
        if on && due() {
            let found = cx.background_executor().spawn(async { check() }).await;
            if let Ok(found) = found {
                let alive = cx.update(|cx| publish(found, cx));
                if alive.is_err() {
                    break;
                }
            }
        }
        cx.background_executor()
            .timer(Duration::from_secs(60 * 60))
            .await;
    })
    .detach();
}

/// Stores the result of a check and redraws the windows if it changed.
pub fn publish(found: Option<Update>, cx: &mut gpui::App) {
    if cx.global::<Available>().0 != found {
        cx.set_global(Available(found));
        cx.refresh_windows();
    }
}

/// `https://github.com/o/r/releases/tag/v1.2.3` → `1.2.3`.
fn version_from_url(url: &str) -> Option<String> {
    let tag = url.split("/releases/tag/").nth(1)?;
    let tag = tag.split(['?', '#', '/']).next()?;
    let version = tag.strip_prefix('v').unwrap_or(tag);
    parse(version).map(|_| version.to_string())
}

fn parse(version: &str) -> Option<(u64, u64, u64)> {
    // Pre-release and build suffixes are ignored: 1.2.3-beta counts as 1.2.3.
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse(candidate), parse(current)), (Some(a), Some(b)) if a > b)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The cache is three lines: check time (Unix seconds), version, release URL.
fn read_cache() -> Option<(u64, String, String)> {
    let text = std::fs::read_to_string(crate::recent::state_dir()?.join(CACHE)).ok()?;
    let mut lines = text.lines();
    let at = lines.next()?.trim().parse().ok()?;
    let version = lines.next().unwrap_or("").trim().to_string();
    let url = lines.next().unwrap_or("").trim().to_string();
    Some((at, version, url))
}

fn write_cache(version: &str, url: &str) {
    let Some(dir) = crate::recent::state_dir() else {
        return;
    };
    let result = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(CACHE), format!("{}\n{version}\n{url}\n", now())));
    if let Err(e) = result {
        log::warn!("could not save the update check: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(
            version_from_url("https://github.com/rk-kabilan/drift/releases/tag/v0.2.0"),
            Some("0.2.0".into())
        );
        assert_eq!(
            version_from_url("https://github.com/o/r/releases/tag/1.10.3-beta.1"),
            Some("1.10.3-beta.1".into())
        );
        assert_eq!(version_from_url("https://github.com/o/r/releases"), None);
        assert_eq!(version_from_url(""), None);
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }
}
