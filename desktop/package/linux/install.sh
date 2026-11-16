#!/bin/sh
# Installs Vesper for this user, without root: the app in ~/.local/share/vesper, a link in ~/.local/bin, the launcher
# entry, the icons, and the vesper:// link handler. Run it again to update. ./install.sh --uninstall removes it all.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
app=$data/vesper
bin=$HOME/.local/bin
entry=$data/applications/vesper.desktop
sizes="16 24 32 48 64 128 256 512"

refresh() {
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$data/applications" || true
  command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
}

if [ "${1:-}" = "--uninstall" ]; then
  rm -rf "$app"
  rm -f "$bin/vesper-desktop" "$entry"
  for s in $sizes; do rm -f "$data/icons/hicolor/${s}x${s}/apps/vesper.png"; done
  refresh
  echo "Vesper is removed. Your settings stay in ${XDG_CONFIG_HOME:-$HOME/.config}/Vesper."
  exit 0
fi

mkdir -p "$app" "$bin" "$data/applications"
# A new name, then a rename: a running Vesper keeps its old file.
cp "$here/vesper-desktop" "$app/vesper-desktop.new"
chmod 755 "$app/vesper-desktop.new"
mv -f "$app/vesper-desktop.new" "$app/vesper-desktop"
for f in README.txt LICENSE.txt THIRD-PARTY.txt; do
  if [ -f "$here/$f" ]; then cp "$here/$f" "$app/$f"; fi
done
ln -sf "$app/vesper-desktop" "$bin/vesper-desktop"
for s in $sizes; do
  mkdir -p "$data/icons/hicolor/${s}x${s}/apps"
  cp "$here/icons/$s.png" "$data/icons/hicolor/${s}x${s}/apps/vesper.png"
done
sed "s|^Exec=.*|Exec=\"$app/vesper-desktop\" %u|" "$here/vesper.desktop" > "$entry"
refresh
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}"
command -v xdg-mime >/dev/null && xdg-mime default vesper.desktop x-scheme-handler/vesper || true
echo "Vesper is installed. Open it from your app menu, or run $bin/vesper-desktop."
