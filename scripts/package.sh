#!/usr/bin/env bash
# Packages Drift for the platform this runs on, passing any options through:
# macOS (.app + .dmg), Linux (.tar.gz, .deb, AppImage) or Windows from Git Bash
# (.zip + installer).
set -euo pipefail
here=$(dirname "$0")
case "$(uname -s)" in
  Darwin) exec "$here/package-macos.sh" "$@" ;;
  Linux) exec "$here/package-linux.sh" "$@" ;;
  MINGW* | MSYS* | CYGWIN*)
    flags=()
    [[ "${1:-}" == "--no-build" ]] && flags=(-NoBuild)
    exec powershell.exe -ExecutionPolicy Bypass -File "$here/package-windows.ps1" ${flags[@]+"${flags[@]}"} ;;
  *) echo "Unsupported platform: $(uname -s)" >&2; exit 1 ;;
esac
