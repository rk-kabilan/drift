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
//!
//! When a newer release is found, Drift downloads it from the release (again
//! with `curl`) and puts it in place of the running copy: the app bundle on
//! macOS, the executable on Windows and Linux, or the AppImage. The running
//! app keeps going; "Restart to Update" starts the new version. Copies that
//! can't replace themselves (installed by a package manager, in a folder Drift
//! can't write to, or a debug build) link to the release page instead.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const LATEST: &str = "https://github.com/rk-kabilan/drift/releases/latest";
const DOWNLOAD: &str = "https://github.com/rk-kabilan/drift/releases/download";
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
    let mut cmd = command("curl");
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
    .stderr(Stdio::null());
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
    cx.set_global(Install::Idle);
    cx.set_global(Available(None));
    cx.background_executor()
        .spawn(async { remove_leftovers() })
        .detach();
    match cached() {
        found if enabled(cx) => publish(found, cx),
        found => cx.set_global(Available(found)),
    }
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

/// Stores the result of a check and redraws the windows if it changed. A new
/// version starts downloading straight away.
pub fn publish(found: Option<Update>, cx: &mut gpui::App) {
    if let Some(update) = &found {
        download(update, false, cx);
    }
    if cx.global::<Available>().0 != found {
        cx.set_global(Available(found));
        cx.refresh_windows();
    }
}

/// Where the in-app update is, shared by every window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    Idle,
    /// Downloading and putting this version in place.
    Downloading(String),
    /// This version replaced the running copy; a restart starts it.
    Ready(String),
    /// Installing this version failed; the release page is offered instead.
    Failed(String),
}

impl gpui::Global for Install {}

/// What the status bar, welcome screen and Settings offer.
pub enum Offer {
    None,
    /// Open the release page: this copy can't update itself, or it failed.
    Page(Update),
    Downloading(String),
    Restart(String),
}

pub fn offer(cx: &gpui::App) -> Offer {
    let available = cx.global::<Available>().0.clone();
    match (cx.global::<Install>(), available) {
        (Install::Ready(v), _) => Offer::Restart(v.clone()),
        (Install::Downloading(v), Some(u)) if *v == u.version => Offer::Downloading(v.clone()),
        (_, Some(u)) => Offer::Page(u),
        (_, None) => Offer::None,
    }
}

/// Whether this copy of Drift can replace itself.
pub fn can_install() -> bool {
    static CAN: OnceLock<bool> = OnceLock::new();
    *CAN.get_or_init(|| target().is_some())
}

/// Downloads and installs `update` in the background, unless that is already
/// done or under way. A failed version is tried again only when `retry` is set
/// (a "Check for Updates" by hand).
pub fn download(update: &Update, retry: bool, cx: &mut gpui::App) {
    if !can_install() {
        return;
    }
    let version = update.version.clone();
    match cx.global::<Install>() {
        Install::Downloading(v) | Install::Ready(v) if *v == version => return,
        Install::Failed(v) if *v == version && !retry => return,
        Install::Ready(_) | Install::Downloading(_) | Install::Failed(_) | Install::Idle => {}
    }
    cx.set_global(Install::Downloading(version.clone()));
    cx.refresh_windows();
    cx.spawn(async move |cx| {
        let v = version.clone();
        let result = cx
            .background_executor()
            .spawn(async move { install(&v) })
            .await;
        cx.update(|cx| {
            let state = match result {
                Ok(()) => {
                    log::info!("installed Drift {version}");
                    Install::Ready(version)
                }
                Err(e) => {
                    log::error!("Could not install Drift {version}: {e}");
                    Install::Failed(version)
                }
            };
            cx.set_global(state);
            cx.refresh_windows();
        })
        .ok();
    })
    .detach();
}

/// Starts the installed version, for a restart. Call it just before quitting:
/// `path` is a repository to open in it.
pub fn relaunch(path: Option<&Path>) {
    let Some(target) = target() else {
        return;
    };
    let mut cmd = if cfg!(target_os = "macos") {
        // -n: a new instance, not this one, which is still quitting.
        let mut cmd = Command::new("open");
        cmd.arg("-n").arg(target.path());
        if let Some(p) = path {
            cmd.arg("--args").arg(p);
        }
        cmd
    } else {
        let mut cmd = Command::new(target.path());
        cmd.args(path);
        cmd
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Err(e) = cmd.spawn() {
        log::error!("could not start the new version of Drift: {e}");
    }
}

/// Where this copy of Drift lives, when it can replace itself.
#[derive(Clone, Debug)]
enum Target {
    /// macOS: the .app bundle.
    App(PathBuf),
    /// Linux: the AppImage file.
    AppImage(PathBuf),
    /// Windows and Linux: the executable.
    Exe(PathBuf),
}

impl Target {
    fn path(&self) -> &Path {
        match self {
            Target::App(p) | Target::AppImage(p) | Target::Exe(p) => p,
        }
    }

    /// The release file this copy updates from.
    fn asset(&self, version: &str) -> String {
        let arch = std::env::consts::ARCH;
        match self {
            Target::App(_) => format!("Drift-{version}.dmg"),
            Target::AppImage(_) => format!("Drift-{version}-{arch}.AppImage"),
            Target::Exe(_) if cfg!(windows) => {
                let arch = if arch == "aarch64" { "arm64" } else { "x64" };
                format!("Drift-{version}-windows-{arch}.zip")
            }
            Target::Exe(_) => format!("drift-{version}-linux-{arch}.tar.gz"),
        }
    }
}

fn target() -> Option<Target> {
    // A debug build would replace itself with a release build.
    if cfg!(debug_assertions) && std::env::var("DRIFT_UPDATE_INSTALL").as_deref() != Ok("1") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let target = if cfg!(target_os = "macos") {
        // Drift.app/Contents/MacOS/drift
        let bundle = exe.parent()?.parent()?.parent()?;
        if bundle.extension()? != "app" {
            return None;
        }
        Target::App(bundle.to_path_buf())
    } else if let Some(image) = std::env::var_os("APPIMAGE").filter(|_| cfg!(target_os = "linux")) {
        Target::AppImage(PathBuf::from(image))
    } else if cfg!(target_os = "linux") && exe.starts_with("/usr") {
        // Installed from a .deb or .rpm: the package manager updates it.
        return None;
    } else {
        Target::Exe(exe)
    };
    writable(target.path().parent()?).then_some(target)
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".drift-write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// A command that opens no console window on Windows.
fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Runs a command, turning a failure into its error output.
fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<String, String> {
    let out = command(program)
        .args(args)
        .output()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!("{program} failed: {err}"))
    }
}

/// Downloads `version` and puts it in place of the running copy. Blocking.
fn install(version: &str) -> Result<(), String> {
    let target = target().ok_or("this copy of Drift can't update itself")?;
    let dir = crate::recent::state_dir()
        .ok_or("no folder for the download")?
        .join("update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = download_and_replace(&target, version, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn download_and_replace(target: &Target, version: &str, dir: &Path) -> Result<(), String> {
    let asset = target.asset(version);
    let file = dir.join(&asset);
    let url = format!("{DOWNLOAD}/v{version}/{asset}");
    run(
        "curl",
        &["-fsSL", "--retry", "2", "--max-time", "900", "-o"]
            .map(std::ffi::OsStr::new)
            .into_iter()
            .chain([file.as_os_str(), url.as_ref()])
            .collect::<Vec<_>>(),
    )?;

    let path = target.path();
    let parent = path.parent().ok_or("no folder")?;
    // The new copy goes next to the old one first, so the swap is a rename on
    // the same disk.
    let staged = parent.join(match target {
        Target::App(_) => ".Drift-update.app",
        _ => ".drift-update",
    });
    remove(&staged);
    match target {
        Target::App(_) => {
            let mount = dir.join("mount");
            std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
            let os = std::ffi::OsStr::new;
            run(
                "hdiutil",
                &[
                    os("attach"),
                    os("-nobrowse"),
                    os("-readonly"),
                    os("-noautoopen"),
                    os("-mountpoint"),
                    mount.as_os_str(),
                    file.as_os_str(),
                ],
            )?;
            let copied = run(
                "ditto",
                &[mount.join("Drift.app").as_os_str(), staged.as_os_str()],
            );
            let _ = run("hdiutil", &[os("detach"), mount.as_os_str(), os("-force")]);
            copied?;
            let plist = staged.join("Contents/Info.plist");
            let got = run(
                "plutil",
                &[
                    os("-extract"),
                    os("CFBundleShortVersionString"),
                    os("raw"),
                    os("-o"),
                    os("-"),
                    plist.as_os_str(),
                ],
            )?;
            if got != version {
                remove(&staged);
                return Err(format!("the download is version {got}, not {version}"));
            }
        }
        Target::AppImage(_) => copy(&file, &staged)?,
        Target::Exe(_) => {
            let unpacked = dir.join("unpacked");
            std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
            // tar ships with Windows 10+ and reads zip files too.
            run(
                "tar",
                &[
                    std::ffi::OsStr::new("-xf"),
                    file.as_os_str(),
                    std::ffi::OsStr::new("-C"),
                    unpacked.as_os_str(),
                ],
            )?;
            let exe = if cfg!(windows) {
                unpacked.join("drift.exe")
            } else {
                let name = asset.trim_end_matches(".tar.gz");
                unpacked.join(name).join("bin/drift")
            };
            if !exe.is_file() {
                return Err(format!("{asset} has no Drift executable"));
            }
            copy(&exe, &staged)?;
        }
    }
    replace(target, &staged)
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn remove(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

/// The old copy is moved aside here, and removed on the next start.
fn old_copy(target: &Target) -> Option<PathBuf> {
    let path = target.path();
    match target {
        Target::App(_) => Some(path.parent()?.join(".Drift-old.app")),
        // Unix replaces a running executable with a rename; Windows can rename
        // a running .exe but not overwrite it.
        Target::Exe(_) if cfg!(windows) => Some(path.with_extension("exe.old")),
        _ => None,
    }
}

/// Swaps the staged copy in. The running app keeps the old files it has open.
fn replace(target: &Target, staged: &Path) -> Result<(), String> {
    let path = target.path();
    let swap = || -> std::io::Result<()> {
        match old_copy(target) {
            Some(old) => {
                remove(&old);
                std::fs::rename(path, &old)?;
                std::fs::rename(staged, path).inspect_err(|_| {
                    // Put the running copy back.
                    let _ = std::fs::rename(&old, path);
                })?;
                if matches!(target, Target::App(_)) {
                    remove(&old);
                }
                Ok(())
            }
            None => std::fs::rename(staged, path),
        }
    };
    swap().map_err(|e| {
        remove(staged);
        format!("could not replace {}: {e}", path.display())
    })
}

/// Removes what an earlier update left behind.
fn remove_leftovers() {
    if let Some(target) = target() {
        if let Some(old) = old_copy(&target) {
            remove(&old);
        }
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
