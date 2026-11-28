//! The welcome screen, shown when no repository is open: open, clone or create a
//! repository, or reopen a recent one. Also the flows behind those actions, which
//! the command palette uses too.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use gpui::{
    div, img, prelude::*, px, Context, Image, ImageFormat, PathPromptOptions, PromptLevel,
    SharedString, Window,
};

use super::picker::PickerItem;
use super::widgets::{icon_sized, tooltip};
use super::workspace::{activate_repo_window, open_window, Workspace};
use crate::git::Repo;
use crate::recent;
use crate::theme::ActiveTheme as _;

/// The Drift mark (a PNG at 3x the 56 px it shows at), shared by every render.
fn logo() -> Arc<Image> {
    static LOGO: OnceLock<Arc<Image>> = OnceLock::new();
    LOGO.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../assets/logo.png").to_vec(),
        ))
    })
    .clone()
}

/// The folder name a clone of `url` gets, as `git clone` would pick it.
fn clone_name(url: &str) -> String {
    let url = url.trim().trim_end_matches(['/', '\\']);
    let url = url.strip_suffix(".git").unwrap_or(url);
    url.rsplit(['/', ':', '\\'])
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("repository")
        .to_string()
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// A button on the update banner: its label and what it does.
type BannerAction = (&'static str, Box<dyn Fn(&mut gpui::App)>);

impl Workspace {
    /// Opens the repository containing `path` in its own window. A window that
    /// already has it comes to the front instead, and the welcome window turns into
    /// the repository's window. A folder that isn't in a repository gets an offer
    /// to initialize one there.
    pub fn open_repo(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match Repo::discover(&path) {
            Ok(repo) => {
                let root = repo.root().to_path_buf();
                if self.repo.is_some() && root == self.root {
                    window.activate_window();
                    return;
                }
                let here = window
                    .window_handle()
                    .downcast::<Workspace>()
                    .filter(|_| self.repo.is_none() && self.busy.is_none());
                let cascade = window.window_bounds().get_bounds();
                // Deferred: other windows can only be looked at once this one's
                // update has finished.
                cx.defer(move |cx| {
                    if activate_repo_window(&root, cx) {
                        return;
                    }
                    let reused = here.is_some_and(|here| {
                        here.update(cx, |this, window, cx| {
                            this.switch_repo(Some(repo), window, cx)
                        })
                        .is_ok()
                    });
                    if !reused {
                        open_window(Some((root, true)), Some(cascade), cx);
                    }
                });
            }
            Err(_) if path.is_dir() => {
                let answer = window.prompt(
                    PromptLevel::Info,
                    &format!("“{}” isn't a git repository", folder_name(&path)),
                    Some("Do you want to initialize a new repository in this folder?"),
                    &["Initialize", "Cancel"],
                    cx,
                );
                cx.spawn_in(window, async move |this, cx| {
                    if matches!(answer.await, Ok(0)) {
                        this.update_in(cx, |this, window, cx| this.init_repo(path, window, cx))
                            .ok();
                    }
                })
                .detach();
            }
            Err(_) => {
                self.recent = recent::remove(&path);
                self.open_failed(format!("{} no longer exists.", path.display()), cx);
            }
        }
    }

    /// Shows why opening failed: on the welcome screen, or in the Output panel
    /// when a repository is open.
    pub(super) fn open_failed(&mut self, message: String, cx: &mut Context<Self>) {
        if self.repo.is_some() {
            self.report_error(message, cx);
        } else {
            self.welcome_error = Some(message);
        }
        cx.notify();
    }

    /// Opens an empty window on the welcome screen.
    pub fn new_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cascade = window.window_bounds().get_bounds();
        cx.defer(move |cx| open_window(None, Some(cascade), cx));
    }

    /// Closes the open repository and shows the welcome screen.
    pub fn close_repo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo.is_none() || self.busy.is_some() {
            return;
        }
        self.confirm_unsaved(window, cx, |this, window, cx| {
            this.switch_repo(None, window, cx)
        });
    }

    fn pick_folder(
        &mut self,
        button: &'static str,
        then: impl FnOnce(&mut Self, PathBuf, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(button.into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = pick.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        then(this, path, window, cx)
                    }
                }
                Ok(Ok(None)) | Err(_) => {}
                Ok(Err(e)) => {
                    this.open_failed(format!("Could not show the folder picker: {e:#}"), cx)
                }
            })
            .ok();
        })
        .detach();
    }

    /// Asks for a folder and opens the repository in it.
    pub fn open_repo_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_folder(
            "Open",
            |this, path, window, cx| this.open_repo(path, window, cx),
            window,
            cx,
        );
    }

    /// Asks for a folder, runs `git init` there and opens it.
    pub fn new_repo_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_folder(
            "Initialize",
            |this, path, window, cx| this.init_repo(path, window, cx),
            window,
            cx,
        );
    }

    fn init_repo(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match Repo::run(&path, &["init".to_string()]) {
            Ok(_) => self.open_repo(path, window, cx),
            Err(e) => self.open_failed(format!("{e:#}"), cx),
        }
    }

    /// Asks for a URL and a parent folder, clones into it and opens the clone.
    pub fn clone_repo_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.ask_text(
            "Repository URL",
            "Enter the URL of the repository to clone, then choose where to put it",
            false,
            None,
            |this, url, window, cx| {
                let url = url.trim().to_string();
                this.pick_folder(
                    "Clone Here",
                    move |this, parent, window, cx| this.clone_into(url, parent, window, cx),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    fn clone_into(
        &mut self,
        url: String,
        parent: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        let name = clone_name(&url);
        let dest = parent.join(&name);
        self.busy = Some(format!("Cloning {name}…"));
        self.welcome_error = None;
        cx.notify();
        let args = vec![
            "clone".to_string(),
            "--".to_string(),
            url,
            dest.to_string_lossy().into_owned(),
        ];
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { Repo::run(&parent, &args) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match result {
                    Ok(_) => this.open_repo(dest, window, cx),
                    Err(e) => this.open_failed(format!("{e:#}"), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// The note under the welcome heading about a newer version: download it,
    /// see it downloading, or restart into it.
    fn render_update_banner(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        use crate::update::Offer;
        let t = cx.theme().clone();
        let (text, action): (String, Option<BannerAction>) = match crate::update::offer(cx) {
            Offer::None => return None,
            Offer::Page(u) => (
                format!("Drift {} is available.", u.version),
                Some(("Download", Box::new(move |cx| cx.open_url(&u.url)))),
            ),
            Offer::Downloading(v) => (format!("Downloading Drift {v}…"), None),
            Offer::Restart(v) => (
                format!("Drift {v} is installed."),
                Some(("Restart", Box::new(super::workspace::restart))),
            ),
        };
        Some(
            div()
                .id("welcome-update")
                .mt(px(12.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(4.))
                .border_1()
                .border_color(t.accent.opacity(0.5))
                .bg(t.accent.opacity(0.08))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon_sized("arrow-down", 14., t.accent))
                .child(div().flex_1().child(text))
                .when_some(action, |d, (label, run)| {
                    d.child(
                        div()
                            .text_color(t.accent)
                            .cursor_pointer()
                            .hover(|s| s.underline())
                            .child(label),
                    )
                    .on_click(move |_, _, cx| run(cx))
                }),
        )
    }

    /// Checks for a newer release now and says what it found.
    pub fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.checking_updates {
            return;
        }
        self.checking_updates = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async { crate::update::check() })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.checking_updates = false;
                cx.notify();
                let current = env!("CARGO_PKG_VERSION");
                match found {
                    Ok(Some(update)) if crate::update::can_install() => {
                        crate::update::publish(Some(update.clone()), cx);
                        // Retry a download that failed earlier.
                        crate::update::download(&update, true, cx);
                        let ready = matches!(
                            cx.global::<crate::update::Install>(),
                            crate::update::Install::Ready(v) if *v == update.version
                        );
                        let (detail, buttons): (String, &[&str]) = if ready {
                            (
                                format!("You have {current}. The new version is installed; restart Drift to start it."),
                                &["Restart Now", "Later"],
                            )
                        } else {
                            (
                                format!("You have {current}. The new version is downloading and installs by itself. Restart Drift when the status bar says it's ready."),
                                &["OK"],
                            )
                        };
                        let answer = window.prompt(
                            PromptLevel::Info,
                            &format!("Drift {} is available", update.version),
                            Some(&detail),
                            buttons,
                            cx,
                        );
                        cx.spawn(async move |_, cx| {
                            if ready && matches!(answer.await, Ok(0)) {
                                cx.update(super::workspace::restart).ok();
                            }
                        })
                        .detach();
                    }
                    Ok(Some(update)) => {
                        crate::update::publish(Some(update.clone()), cx);
                        let answer = window.prompt(
                            PromptLevel::Info,
                            &format!("Drift {} is available", update.version),
                            Some(&format!("You have {current}. Download the new version?")),
                            &["Download", "Later"],
                            cx,
                        );
                        cx.spawn(async move |_, cx| {
                            if matches!(answer.await, Ok(0)) {
                                cx.update(|cx| cx.open_url(&update.url)).ok();
                            }
                        })
                        .detach();
                    }
                    Ok(None) => {
                        crate::update::publish(None, cx);
                        // Only one button, so the answer isn't needed.
                        drop(window.prompt(
                            PromptLevel::Info,
                            "Drift is up to date",
                            Some(&format!("You have the latest version, {current}.")),
                            &["OK"],
                            cx,
                        ));
                    }
                    Err(()) => {
                        // Only one button, so the answer isn't needed.
                        drop(window.prompt(
                            PromptLevel::Warning,
                            "Couldn't check for updates",
                            Some("The releases page couldn't be reached. Check your connection and try again."),
                            &["OK"],
                            cx,
                        ));
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Lists recent repositories in a picker (the command palette's "Open Recent").
    pub fn open_recent_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recent = recent::load();
        let paths: Vec<PathBuf> = self
            .recent
            .iter()
            .filter(|p| self.repo.is_none() || **p != self.root)
            .cloned()
            .collect();
        let items = paths
            .iter()
            .map(|p| {
                let haystack = recent::display(p);
                let label_start = haystack
                    .rfind(['/', '\\'])
                    .map_or(0, |i| i + 1)
                    .min(haystack.len());
                PickerItem {
                    haystack,
                    label_start,
                    note: String::new(),
                    icon: "repo",
                    icon_color: None,
                    badge: None,
                    section: None,
                }
            })
            .collect();
        self.pick_list(
            "Open a recent repository",
            "No recent repositories.",
            items,
            move |this, ix, window, cx| {
                if let Some(path) = paths.get(ix).cloned() {
                    this.open_repo(path, window, cx)
                }
            },
            window,
            cx,
        );
    }

    pub(super) fn render_welcome(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = cx.theme().clone();
        let busy = self.busy.is_some();

        let heading = |text: &'static str| {
            div()
                .pt(px(18.))
                .pb(px(6.))
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child(text)
        };
        let row = |id: SharedString| {
            div()
                .id(id)
                .h(px(30.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(5.))
                .when(!busy, |d| d.cursor_pointer().hover(|s| s.bg(t.hover)))
        };
        let action = |id: &'static str,
                      icon: &'static str,
                      label: &'static str,
                      keys: Option<String>,
                      cx: &mut Context<Self>,
                      run: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            row(id.into())
                .child(icon_sized(icon, 16., t.accent))
                .child(div().flex_1().text_color(t.accent).child(label))
                .children(keys.map(|k| div().text_size(px(12.)).text_color(t.faint).child(k)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.busy.is_none() {
                        run(this, window, cx)
                    }
                }))
        };

        let recent_rows: Vec<gpui::AnyElement> = self
            .recent
            .iter()
            .enumerate()
            .map(|(ix, path)| {
                let group: SharedString = format!("recent-{ix}").into();
                let open = path.clone();
                let forget = path.clone();
                row(group.clone())
                    .group(group.clone())
                    .child(icon_sized("repo", 16., t.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_baseline()
                            .gap(px(8.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(div().flex_none().text_color(t.fg).child(folder_name(path)))
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_size(px(12.))
                                    .text_color(t.muted)
                                    .child(recent::display(path)),
                            ),
                    )
                    .child(
                        div()
                            .id(("recent-remove", ix))
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .w(px(20.))
                            .h(px(20.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .hover(|s| s.bg(t.hover))
                            .tooltip(tooltip("Remove from Recent"))
                            .child(icon_sized("close", 14., t.muted))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.recent = recent::remove(&forget);
                                cx.notify();
                            })),
                    )
                    .tooltip(tooltip(path.display().to_string()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_repo(open.clone(), window, cx)
                    }))
                    .into_any_element()
            })
            .collect();

        let column = div()
            .w(px(520.))
            .max_w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .pb(px(8.))
                    .child(img(logo()).w(px(56.)).h(px(54.)).flex_none())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(26.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Drift"),
                            )
                            .child(div().text_color(t.muted).child("A fast Git desktop client")),
                    ),
            )
            .children(self.render_update_banner(cx))
            .children(self.welcome_error.clone().map(|e| {
                div()
                    .mt(px(12.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(t.error.opacity(0.5))
                    .bg(t.error.opacity(0.08))
                    .text_color(t.error)
                    .child(e)
            }))
            .children(
                self.busy
                    .clone()
                    .map(|b| div().mt(px(12.)).text_color(t.muted).child(b)),
            )
            .child(heading("START"))
            .child(action(
                "welcome-open",
                "open",
                "Open Repository…",
                super::shortcuts::label("OpenRepository", cx),
                cx,
                Self::open_repo_dialog,
            ))
            .child(action(
                "welcome-clone",
                "cloud",
                "Clone Repository…",
                None,
                cx,
                Self::clone_repo_dialog,
            ))
            .child(action(
                "welcome-new",
                "plus",
                "New Repository…",
                None,
                cx,
                Self::new_repo_dialog,
            ))
            .child(heading("RECENT"))
            .when(recent_rows.is_empty(), |d| {
                d.child(
                    div()
                        .px(px(8.))
                        .text_color(t.muted)
                        .child("Repositories you open will appear here."),
                )
            })
            .children(recent_rows);

        div()
            .id("welcome")
            .flex_1()
            .min_h_0()
            .mx(px(6.))
            .mb(px(6.))
            .bg(t.panel)
            .border_1()
            .border_color(t.border)
            .rounded(px(8.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .px(px(16.))
            .pt(px(64.))
            .pb(px(24.))
            .child(column)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::clone_name;

    #[test]
    fn clone_names() {
        assert_eq!(clone_name("https://github.com/rk/drift.git"), "drift");
        assert_eq!(clone_name("https://github.com/rk/drift/"), "drift");
        assert_eq!(clone_name("git@github.com:rk/drift.git"), "drift");
        assert_eq!(clone_name("git@host:drift"), "drift");
        assert_eq!(clone_name("/srv/git/drift.git"), "drift");
        assert_eq!(clone_name("C:\\repos\\drift"), "drift");
        assert_eq!(clone_name(""), "repository");
    }
}
