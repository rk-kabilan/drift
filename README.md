# Drift

A fast Git desktop client written in Rust with [GPUI](https://gpui.rs), the GPU-accelerated UI framework behind Zed. It runs on macOS, Windows and Linux.

Drift does one thing: review and commit changes. There is no editor, file search or extension system.

## What it does

- **Changes pane.** Commit message box, a Commit button with a menu (Commit Staged, Commit All, Amend, Commit & Push), and **Staged Changes** / **Changes** groups. Each file shows its folder and a status letter: `M` modified, `A` added, `D` deleted, `R` renamed, `U` untracked, `C` conflicted. Hover a file to open it, discard its changes, or stage/unstage it. Hover a group header to stage, unstage or discard everything in it.
- **Diff pane.** Side by side (HEAD or index on the left, working tree on the right) or inline, with line numbers, red/green line backgrounds, word-level highlights of what changed inside a line, light syntax colouring, and unchanged regions folded away (click a fold to expand it). Arrows jump between changes.
- **Status bar.** Repository, branch with a `*` when the tree is dirty, commits to pull/push (click to sync), and blame for the cursor line: author, age, commit message and short hash. The same blame shows faintly at the end of the cursor line.
- **Title bar.** Repository and branch only.
- **Light and dark themes.** Follows the OS; the sun/moon button in the status bar switches manually, and `DRIFT_THEME=light|dark` pins one.

Reads (status, file contents, blame, ahead/behind) use libgit2. Writes (stage, unstage, discard, commit, push, pull) run the `git` command, so your hooks, signing and credential helpers apply exactly as in a terminal. Drift refreshes when its window gains focus and every few seconds while it is in front.

## Build and run

You need a recent stable Rust toolchain and `git` on your `PATH`.

```sh
cargo run --release              # opens the repository containing the current folder
cargo run --release -- ~/code/x  # or a given path
```

Started anywhere else (or from the Dock, Start menu or app launcher), Drift shows a welcome screen: open a repository folder, clone one, create a new one, or pick one of the last ten you opened. Each repository opens in its own window; the welcome window becomes the first one you open from it. The recent list is kept in `~/Library/Application Support/drift/` on macOS, `%APPDATA%\drift\` on Windows and `~/.local/state/drift/` on Linux.

**Linux** also needs the usual GPUI system libraries, for example on Debian/Ubuntu:

```sh
sudo apt install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev \
  libx11-xcb-dev libxcb1-dev libfontconfig-dev pkg-config clang
```

**macOS** needs Xcode command line tools; **Windows** needs the MSVC build tools.

## Packaging

One command per platform builds a release and packages it into `target/release/<platform>/`. Each package has to be built on its own OS.

| Platform | Command | Output |
| --- | --- | --- |
| macOS | `scripts/package-macos.sh` | `Drift.app` and `Drift-<version>.dmg` (drag to Applications) |
| Linux | `scripts/package-linux.sh` | `drift-<version>-linux-<arch>.tar.gz` (with `install.sh`), a `.deb` when `dpkg-deb` exists, an `.rpm` when `rpmbuild` exists, an AppImage when `appimagetool` is on `PATH` |
| Windows | `powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1` | `Drift-<version>-windows-<arch>.zip` (portable) and `Drift-<version>-setup-<arch>.exe` when [Inno Setup 6](https://jrsoftware.org/isinfo.php) is installed |

`scripts/package.sh` picks the right one for the machine it runs on (on Windows, from Git Bash). Options:

- All scripts: `--no-build` (`-NoBuild` on Windows) packages the existing release binary.
- macOS: `--no-dmg` for the app alone; `--universal` for one app that runs on Apple silicon and Intel Macs (first `rustup target add aarch64-apple-darwin x86_64-apple-darwin`).
- Linux: set `DRIFT_MAINTAINER="Name <email>"` for the `.deb` maintainer field. Build on the oldest distro you want to support, since the binary needs that glibc or newer.

The macOS app is signed ad hoc, which runs on your own Mac. To share builds without warnings, sign and notarize the app with an Apple Developer ID, and sign the Windows exe and installer with `signtool`.

### Releasing

`.github/workflows/release.yml` builds every package on GitHub's macOS, Windows and Linux machines and publishes them as a GitHub Release. To release, set `version` in `Cargo.toml`, commit, and push a matching tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The workflow stops if the tag and `Cargo.toml` disagree. Running it by hand from the Actions tab builds the packages as downloadable artifacts without publishing anything.

### Updates

Once a day Drift checks the repository's latest GitHub Release (with the system's `curl`, so the app carries no HTTP code). When a newer version exists, the status bar and the welcome screen show "Drift x.y.z available", linking to the download page. The update icon in the status bar, "Check Now" in Settings and "Settings: Check for Updates…" in the command palette check right away. Installing the new package replaces the old version; settings and recent repositories are kept. The daily check can be turned off in Settings; `DRIFT_UPDATE_CHECK=0` or `1` overrides that, and debug builds don't check unless it is `1`. Release downloads must be public for this to work.

### App icon

The icon lives in `assets/app-icon/` (`drift.svg` is the master; `drift-macos.svg` is the macOS version on Apple's icon grid). `build.rs` embeds `windows/drift.ico` and the "Drift" product name into `drift.exe`; the macOS script puts `macos/drift.icns` in the app; the Linux packages install the hicolor icons and `drift.desktop`, whose name matches the window's app id (`drift`).

## Settings

Open Settings with `Cmd+,` / `Ctrl+,`, the gear in the status bar, or "Settings: Open Settings" in the command palette.

- **General**: theme (System, Light or Dark; the sun/moon button in the status bar switches too) and the automatic update check.
- **Keyboard Shortcuts**: every action that can have a shortcut. Click Change and press the new keys (Esc cancels), Remove to leave an action without one, Reset to go back to the default. Shortcuts need Ctrl, Alt or Cmd, or a function key. Two actions with the same keys are flagged.

Settings are saved to `settings` in the state folder (macOS `~/Library/Application Support/drift/`, Windows `%APPDATA%\drift\`, Linux `~/.local/state/drift/`), as `key=value` lines:

```
theme=dark
check_updates=true
key.CommandPalette=ctrl-k
```

`DRIFT_THEME=light|dark` still pins the theme a window starts with.

## Keyboard

Defaults (change them in Settings → Keyboard Shortcuts):

| Keys | Action |
| --- | --- |
| `Ctrl+Enter` / `Cmd+Enter` | Commit staged changes |
| `Enter`, `Shift+Enter` | New line in the message |
| `F7` / `Shift+F7` | Next / previous change |
| `F5`, `Ctrl+R` / `Cmd+R` | Refresh |
| `Ctrl+O` / `Cmd+O` | Open a repository in a new window (or bring its window forward) |
| `Ctrl+Shift+N` / `Cmd+Shift+N` | New window |
| `Ctrl+Shift+P` / `Cmd+Shift+P`, `F1` | Command palette |
| `Ctrl+P` / `Cmd+P` | Go to file |
| `Ctrl+,` / `Cmd+,` | Settings |
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
| `scripts/` | Packaging scripts for macOS, Linux and Windows |
| `src/update.rs` | Daily check for a newer release |
| `src/settings.rs` | Settings file: theme, update check, shortcut overrides |
| `src/ui/shortcuts.rs` | Shortcut table, defaults and rebinding |
| `src/ui/settings_panel.rs` | The Settings panel |
| `.github/workflows/release.yml` | Builds and publishes a release when a `v*` tag is pushed |
| `patches/xattr` | Local copy of `xattr` 0.2.3 (pulled in by GPUI) fixed for current `libc` on Linux |

Run the tests with `cargo test`.
