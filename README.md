# drift

A fast, git-only desktop client written in Rust with [GPUI](https://gpui.rs), the GPU-accelerated UI framework behind Zed. It runs on macOS, Windows and Linux.

drift does one thing: review and commit changes. There is no editor, file search or extension system.

## What it does

- **Changes pane.** Commit message box, a Commit button with a menu (Commit Staged, Commit All, Amend, Commit & Push), and **Staged Changes** / **Changes** groups. Each file shows its folder and a status letter: `M` modified, `A` added, `D` deleted, `R` renamed, `U` untracked, `C` conflicted. Hover a file to open it, discard its changes, or stage/unstage it. Hover a group header to stage, unstage or discard everything in it.
- **Diff pane.** Side by side (HEAD or index on the left, working tree on the right) or inline, with line numbers, red/green line backgrounds, word-level highlights of what changed inside a line, light syntax colouring, and unchanged regions folded away (click a fold to expand it). Arrows jump between changes.
- **Status bar.** Repository, branch with a `*` when the tree is dirty, commits to pull/push (click to sync), and blame for the cursor line: author, age, commit message and short hash. The same blame shows faintly at the end of the cursor line.
- **Title bar.** Repository and branch only.
- **Light and dark themes.** Follows the OS; the sun/moon button in the status bar switches manually, and `DRIFT_THEME=light|dark` pins one.

Reads (status, file contents, blame, ahead/behind) use libgit2. Writes (stage, unstage, discard, commit, push, pull) run the `git` command, so your hooks, signing and credential helpers apply exactly as in a terminal. drift refreshes when its window gains focus and every few seconds while it is in front.

## Build and run

You need a recent stable Rust toolchain and `git` on your `PATH`.

```sh
cargo run --release              # opens the repository containing the current folder
cargo run --release -- ~/code/x  # or a given path
```

Started anywhere else (or from the Dock, Start menu or app launcher), drift shows a welcome screen: open a repository folder, clone one, create a new one, or pick one of the last ten you opened. Each repository opens in its own window; the welcome window becomes the first one you open from it. The recent list is kept in `~/Library/Application Support/drift/` on macOS, `%APPDATA%\drift\` on Windows and `~/.local/state/drift/` on Linux.

**Linux** also needs the usual GPUI system libraries, for example on Debian/Ubuntu:

```sh
sudo apt install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev \
  libx11-xcb-dev libxcb1-dev libfontconfig-dev pkg-config clang
```

**macOS** needs Xcode command line tools; **Windows** needs the MSVC build tools.

## App icon and packaging

The icon lives in `assets/app-icon/` (`drift.svg` is the master; `drift-macos.svg` is the macOS version on Apple's icon grid).

- **Windows:** `build.rs` embeds `windows/drift.ico` (16 to 256 px) into `drift.exe`, so Explorer, the taskbar and Alt+Tab show it with a plain `cargo build`.
- **macOS:** a bare binary has no Dock icon. Build an app bundle that uses `macos/drift.icns`:
  ```sh
  cargo install cargo-bundle
  cargo bundle --release            # target/release/bundle/osx/drift.app
  ```
- **Linux:** desktops find the icon through `drift.desktop`. The window's app id (and X11 `WM_CLASS`) is `drift`, which matches that file. Use `cargo bundle --release --format deb` for a `.deb`, or install by hand:
  ```sh
  install -Dm755 target/release/drift ~/.local/bin/drift
  cp -r assets/app-icon/linux/hicolor ~/.local/share/icons/
  install -Dm644 assets/app-icon/linux/drift.desktop ~/.local/share/applications/drift.desktop
  ```

## Keyboard

| Keys | Action |
| --- | --- |
| `Ctrl+Enter` / `Cmd+Enter` | Commit staged changes |
| `Enter`, `Shift+Enter` | New line in the message |
| `F7` / `Shift+F7` | Next / previous change |
| `F5`, `Ctrl+R` / `Cmd+R` | Refresh |
| `Ctrl+O` / `Cmd+O` | Open a repository in a new window (or bring its window forward) |
| `Ctrl+Shift+N` / `Cmd+Shift+N` | New window |
| `Ctrl+Q` / `Cmd+Q` | Quit |

## Layout of the code

| Path | Contents |
| --- | --- |
| `src/git/` | Repository access: status, contents, blame, ahead/behind, and git command writes |
| `src/diff.rs` | Line diff model: hunks, folds, side-by-side and inline rows, word-level changes |
| `src/syntax.rs` | Small language-agnostic highlighter |
| `src/theme.rs` | Light and dark palettes |
| `src/ui/` | GPUI views: workspace, changes pane, diff pane, status bar, commit message input, dialog |
| `assets/icons/` | SVG icons, compiled into the binary |
| `assets/app-icon/` | App icon for macOS (`.icns`), Windows (`.ico`, embedded by `build.rs`) and Linux (hicolor PNGs, SVG, `.desktop`) |
| `patches/xattr` | Local copy of `xattr` 0.2.3 (pulled in by GPUI) fixed for current `libc` on Linux |

Run the tests with `cargo test`.
