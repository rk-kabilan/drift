#!/usr/bin/env bash
# Packages Drift for Linux. Everything lands in target/release/linux/:
#
#   drift-<version>-linux-<arch>.tar.gz   binary, icons, launcher entry and install.sh
#   drift_<version>_<arch>.deb            when dpkg-deb is available (Debian, Ubuntu)
#   drift-<version>-1.<arch>.rpm          when rpmbuild is available (Fedora, RHEL, openSUSE)
#   Drift-<version>-<arch>.AppImage       when appimagetool is on PATH
#
#   scripts/package-linux.sh              build, then package
#   scripts/package-linux.sh --no-build   package the existing target/release/drift
#
# Build on the oldest distro you want to support: the binary needs a glibc at
# least as new as the one it was built against.
set -euo pipefail

cd "$(dirname "$0")/.."
[[ "$(uname)" == Linux ]] || { echo "This script only runs on Linux." >&2; exit 1; }

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
ARCH=$(uname -m)
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64 | arm64) DEB_ARCH=arm64 ;;
  *) DEB_ARCH=$ARCH ;;
esac
MAINTAINER=${DRIFT_MAINTAINER:-"Drift <drift@localhost>"}
BIN=target/release/drift
OUT=target/release/linux
ICONS=assets/app-icon/linux

if [[ "${1:-}" != "--no-build" ]]; then
  cargo build --release
fi
[[ -x "$BIN" ]] || { echo "$BIN is missing; build it first." >&2; exit 1; }

rm -rf "$OUT"
mkdir -p "$OUT"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# Lays out bin/ and share/ (applications, hicolor icons) under $1.
install_tree() {
  local prefix=$1 dir
  install -Dm755 "$BIN" "$prefix/bin/drift"
  install -Dm644 "$ICONS/drift.desktop" "$prefix/share/applications/drift.desktop"
  for dir in "$ICONS"/hicolor/*/; do
    install -Dm644 "$dir"apps/drift.* -t "$prefix/share/icons/hicolor/$(basename "$dir")/apps"
  done
}

# ---- tar.gz with an installer script ----
NAME=drift-$VERSION-linux-$ARCH
install_tree "$WORK/$NAME"
cat > "$WORK/$NAME/install.sh" <<'INSTALL'
#!/bin/sh
# Installs Drift for this user (~/.local), or elsewhere: sudo ./install.sh /usr/local
set -e
PREFIX=${1:-$HOME/.local}
cd "$(dirname "$0")"
mkdir -p "$PREFIX"
cp -R bin share "$PREFIX/"
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
fi
echo "Installed Drift in $PREFIX. Start it from your app launcher, or run: drift"
INSTALL
chmod 755 "$WORK/$NAME/install.sh"
tar -C "$WORK" -czf "$OUT/$NAME.tar.gz" "$NAME"
echo "Built $OUT/$NAME.tar.gz"

# ---- .deb ----
if command -v dpkg-deb >/dev/null 2>&1; then
  DEB=$WORK/deb
  install_tree "$DEB/usr"
  mkdir -p "$DEB/DEBIAN"
  cat > "$DEB/DEBIAN/control" <<CONTROL
Package: drift
Version: $VERSION
Section: devel
Priority: optional
Architecture: $DEB_ARCH
Depends: git, libc6, zlib1g, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libvulkan1, libfontconfig1, libfreetype6, libx11-6, libx11-xcb1, libxcb1
Installed-Size: $(du -sk "$DEB/usr" | cut -f1)
Maintainer: $MAINTAINER
Homepage: https://github.com/rk-kabilan/drift
Description: Fast, git-only desktop client
 Drift reviews and commits changes: a changes pane, side by side and inline
 diffs, blame, branches, stashes and remotes, with nothing else in the way.
CONTROL
  dpkg-deb --root-owner-group --build "$DEB" "$OUT/drift_${VERSION}_${DEB_ARCH}.deb" >/dev/null
  echo "Built $OUT/drift_${VERSION}_${DEB_ARCH}.deb"
else
  echo "Skipped .deb: dpkg-deb not found."
fi

# ---- .rpm ----
if command -v rpmbuild >/dev/null 2>&1; then
  RPM=$WORK/rpm
  install_tree "$RPM/root/usr"
  mkdir -p "$RPM/SPECS"
  # Library names differ between distros, so dependencies are by soname. rpm
  # adds the libraries drift links to; these are the ones gpui loads at runtime.
  cat > "$RPM/SPECS/drift.spec" <<SPEC
%define debug_package %{nil}
%global __os_install_post %{nil}
Name: drift
Version: $VERSION
Release: 1
Summary: Fast, git-only desktop client
License: MIT
URL: https://github.com/rk-kabilan/drift
Requires: git
Requires: libvulkan.so.1()(64bit)
Requires: libwayland-client.so.0()(64bit)
Requires: libfontconfig.so.1()(64bit)
Requires: libfreetype.so.6()(64bit)
Requires: libX11-xcb.so.1()(64bit)

%description
Drift reviews and commits changes: a changes pane, side by side and inline
diffs, blame, branches, stashes and remotes, with nothing else in the way.

%install
cp -a $RPM/root/. %{buildroot}/

%files
/usr/bin/drift
/usr/share/applications/drift.desktop
/usr/share/icons/hicolor/*/apps/drift.*
SPEC
  rpmbuild --quiet -bb --define "_topdir $RPM" --target "$ARCH" "$RPM/SPECS/drift.spec"
  cp "$RPM"/RPMS/*/drift-*.rpm "$OUT/"
  echo "Built $OUT/$(basename "$RPM"/RPMS/*/drift-*.rpm)"
else
  echo "Skipped .rpm: rpmbuild not found (dnf install rpm-build, or apt install rpm)."
fi

# ---- AppImage ----
if command -v appimagetool >/dev/null 2>&1; then
  APPDIR=$WORK/Drift.AppDir
  install_tree "$APPDIR/usr"
  cp "$ICONS/drift.desktop" "$APPDIR/drift.desktop"
  cp "$ICONS/hicolor/256x256/apps/drift.png" "$APPDIR/drift.png"
  ln -s usr/bin/drift "$APPDIR/AppRun"
  ARCH=$ARCH appimagetool --no-appstream "$APPDIR" "$OUT/Drift-$VERSION-$ARCH.AppImage" >/dev/null
  echo "Built $OUT/Drift-$VERSION-$ARCH.AppImage"
else
  echo "Skipped AppImage: appimagetool not found (https://github.com/AppImage/appimagetool)."
fi
