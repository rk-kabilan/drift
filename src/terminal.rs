//! A shell running in a pseudo terminal, for the Terminal tab of the bottom
//! panel. `alacritty_terminal` does the work: it starts the shell (openpty on
//! macOS and Linux, ConPTY on Windows), reads its output on a thread and keeps
//! the screen grid with its scrollback. The view is in `ui::terminal_view`.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, Notify, OnResize as _, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::Config;
use alacritty_terminal::tty;
use alacritty_terminal::Term;
use anyhow::{Context as _, Result};

/// Lines kept above the screen. VS Code's default; each line costs
/// columns × 24 bytes, and only once it has scrolled off.
pub const SCROLLBACK: usize = 1_000;

/// Sends the terminal's events to the view.
#[derive(Clone)]
pub struct Listener(async_channel::Sender<Event>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        // The view drains the channel; once it is gone, events have no reader.
        let _ = self.0.try_send(event);
    }
}

pub struct Terminal {
    pub term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    pub events: async_channel::Receiver<Event>,
    size: WindowSize,
}

/// The screen size in cells, and one cell's size in pixels.
pub fn window_size(cols: usize, rows: usize, cell_w: f32, cell_h: f32) -> WindowSize {
    WindowSize {
        num_cols: cols.clamp(2, u16::MAX as usize) as u16,
        num_lines: rows.clamp(1, u16::MAX as usize) as u16,
        cell_width: cell_w.round().max(1.) as u16,
        cell_height: cell_h.round().max(1.) as u16,
    }
}

impl Terminal {
    /// Starts a shell in `cwd`. `shell` is a command line from the settings
    /// ("/bin/bash -l"); without one the user's login shell starts (PowerShell
    /// on Windows).
    pub fn spawn(cwd: &Path, shell: Option<&str>, size: WindowSize) -> Result<Self> {
        let shell = shell.map(str::split_whitespace).and_then(|mut words| {
            let program = words.next()?.to_string();
            Some(tty::Shell::new(
                program,
                words.map(str::to_string).collect(),
            ))
        });
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "Drift".to_string());
        #[allow(clippy::needless_update)]
        let options = tty::Options {
            shell,
            working_directory: Some(cwd.to_path_buf()),
            drain_on_exit: false,
            env,
            #[cfg(windows)]
            escape_args: true,
            ..Default::default()
        };
        let pty = tty::new(&options, size, 0).context("could not start the shell")?;

        let (tx, events) = async_channel::unbounded();
        let listener = Listener(tx);
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Default::default()
        };
        let dims = TermSize::new(size.num_cols as usize, size.num_lines as usize);
        let term = Arc::new(FairMutex::new(Term::new(config, &dims, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)
            .context("could not start the terminal")?;
        let notifier = Notifier(event_loop.channel());
        event_loop.spawn();
        Ok(Self {
            term,
            notifier,
            events,
            size,
        })
    }

    /// Sends input to the shell.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.notifier.notify(bytes);
    }

    pub fn size(&self) -> WindowSize {
        self.size
    }

    pub fn resize(&mut self, size: WindowSize) {
        let same = |a: WindowSize, b: WindowSize| {
            (a.num_cols, a.num_lines, a.cell_width, a.cell_height)
                == (b.num_cols, b.num_lines, b.cell_width, b.cell_height)
        };
        if same(size, self.size) {
            return;
        }
        self.size = size;
        self.notifier.on_resize(size);
        let dims = TermSize::new(size.num_cols as usize, size.num_lines as usize);
        self.term.lock().resize(dims);
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Ends the reader thread, which closes the pty; the shell gets SIGHUP.
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}
