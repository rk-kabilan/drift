# Drift

A fast Git desktop client written in Rust with [GPUI](https://gpui.rs), the GPU-accelerated UI framework behind Zed. It runs on macOS, Windows and Linux.

Inspired by VS Code: the layout follows its Source Control view.

Drift does one thing: review and commit changes. There is no editor, file search or extension system.

## What it does

- **Changes pane.** Commit message box, a Commit button with a menu (Commit Staged, Commit All, Amend, Commit & Push), and **Staged Changes** / **Changes** groups. Each file shows its folder and a status letter: `M` modified, `A` added, `D` deleted, `R` renamed, `U` untracked, `C` conflicted. Hover a file to open it, discard its changes, or stage/unstage it. Hover a group header to stage, unstage or discard everything in it. Cmd-click (Ctrl-click on Windows and Linux) or Shift-click picks several files; right-click for Open Changes, Stage, Unstage, Discard and Reveal in Finder (File Explorer on Windows, the file manager on Linux).
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

Run the tests with `cargo test`.

## Installing on macOS

The macOS download isn't signed with an Apple Developer ID or notarized yet, so the first time you open it macOS says "Drift Not Opened: Apple could not verify Drift is free of malware". To open it anyway, after dragging Drift to Applications, either:

- run `xattr -dr com.apple.quarantine /Applications/Drift.app` in Terminal, then open Drift as usual, or
- try to open Drift once, then go to **System Settings → Privacy & Security** and click **Open Anyway** next to the Drift message.

You only need to do this once per version. The permanent fix is an Apple Developer account ($99 a year): with a Developer ID certificate, the release workflow can sign and notarize the app so it opens without the warning.

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

`.github/workflows/release.yaml` builds every package on GitHub's macOS, Windows and Linux machines and publishes them as a GitHub Release. To release, set `version` in `Cargo.toml`, commit, and push a matching tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The tests run first, and the packages build only if they pass. The workflow stops if the tag and `Cargo.toml` disagree. Running it by hand from the Actions tab also publishes the `Cargo.toml` version (tagging that commit); untick **Publish** to only build the packages as downloadable artifacts.

## Settings

Open Settings with `Cmd+,` / `Ctrl+,`, the gear in the status bar, or "Settings: Open Settings" in the command palette.

- **General**: theme (System, Light or Dark; the sun/moon button in the status bar switches too), word wrap, the history graph, background fetch (on by default, every 5 seconds; the interval can be changed or fetching turned off) and the automatic update check.
- **Keyboard Shortcuts**: every action that can have a shortcut. Click Change and press the new keys (Esc cancels), Remove to leave an action without one, Reset to go back to the default. Shortcuts need Ctrl, Alt or Cmd, or a function key. Two actions with the same keys are flagged.

Settings are saved to `settings` in the state folder (macOS `~/Library/Application Support/drift/`, Windows `%APPDATA%\drift\`, Linux `~/.local/state/drift/`), as `key=value` lines:

```
theme=dark
check_updates=true
word_wrap=false
show_graph=true
auto_fetch=true
auto_fetch_interval=5
panel_open=false
panel_tab=output
panel_height=240
key.CommandPalette=ctrl-k
```

`terminal_shell=/bin/bash -l` (a command line) picks the terminal's shell; without it the terminal starts your login shell (PowerShell on Windows).

`DRIFT_THEME=light|dark` still pins the theme a window starts with.

## Output and Terminal

The panel under the diff has two tabs, as in VS Code:

- **Output** lists the git commands Drift runs, with how long each took, and every error and warning. A failed operation opens the panel on Output and shows the message in the status bar for a few seconds; click it to jump to Output. Hints such as "No staged changes" only show in the status bar and Output. The log keeps the last 2,000 entries (at most 512 KB), and tokens in remote URLs are masked.
- **Terminal** starts a shell in the repository root the first time you open it. `+` adds more shells; they are listed beside the terminal. While the terminal has focus, keys go to the shell except the panel shortcuts, the command palette, Go to File and Quit. Copy and paste with `Cmd+C` / `Cmd+V` on macOS and `Ctrl+Shift+C` / `Ctrl+Shift+V` elsewhere (on Windows `Ctrl+C` copies when text is selected). `Cmd+K` clears it on macOS. Scrollback keeps 1,000 lines.

Drag the panel's top edge to resize it (double-click resets it); the arrow button maximizes it.

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
| `Ctrl+W` / `Cmd+W` | Close the diff tab, or the window when no diff is open |
| `Ctrl+Shift+W` / `Cmd+Shift+W` | Close the window |
| `Ctrl+Shift+P` / `Cmd+Shift+P`, `F1` | Command palette |
| `Ctrl+P` / `Cmd+P` | Go to file |
| `Alt+Z` / `Option+Z` | Toggle word wrap |
| `Ctrl+Shift+G` / `Cmd+Shift+G` | Show or hide the history graph |
| `Ctrl+J` / `Cmd+J` | Show or hide the bottom panel |
| `` Ctrl+` `` | Open, focus or hide the terminal |
| `` Ctrl+Shift+` `` | New terminal |
| `Ctrl+Shift+U` / `Cmd+Shift+U` | Show Output |
| `Ctrl+,` / `Cmd+,` | Settings |
| `Ctrl+Q` / `Cmd+Q` | Quit |

## License

Drift is licensed under the [Apache License 2.0](LICENSE). You can use, fork and modify it under that license's terms, which include keeping the copyright and [NOTICE](NOTICE). The license does not grant use of the Drift name or logo.

This repository does not accept outside contributions. Fork it to make your own changes.
