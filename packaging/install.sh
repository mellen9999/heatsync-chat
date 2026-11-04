#!/bin/sh
# puts heatsync chat in your home folder (no root): the app, its menu entry and icon.
#   ./install.sh              install it (run from the unpacked release folder; again later to reinstall)
#   ./install.sh --uninstall  take those same files out again
# the app updates itself from then on (the ↑ button on its tab bar)
set -eu

usage() {
  echo "usage: ./install.sh [--uninstall]"
}
case "$#:${1:-}" in
  0:) mode=install ;;
  1:--uninstall) mode=uninstall ;;
  1:-h | 1:--help) usage; exit 0 ;;
  *) usage >&2; exit 2 ;;
esac

[ -n "${HOME:-}" ] || { echo "can't find your home folder (HOME is empty)" >&2; exit 1; }
# the spec says a relative XDG_DATA_HOME is wrong: ignore it
case "${XDG_DATA_HOME:-}" in /*) data=$XDG_DATA_HOME ;; *) data=$HOME/.local/share ;; esac
app="$HOME/.local/bin/heatsync-chat"
entry="$data/applications/heatsync-chat.desktop"
icon48="$data/icons/hicolor/48x48/apps/heatsync-chat.png"
icon96="$data/icons/hicolor/96x96/apps/heatsync-chat.png"

if [ "$mode" = uninstall ]; then
  gone=0
  for f in "$app" "$entry" "$icon48" "$icon96"; do
    if [ -e "$f" ] || [ -L "$f" ]; then
      rm -f -- "$f"
      echo "removed $f"
      gone=$((gone + 1))
    fi
  done
  if [ "$gone" -eq 0 ]; then
    echo "nothing to remove: heatsync chat isn't installed here"
  else
    echo "done: heatsync chat is gone. your settings and chat history are still kept in ~/.config/heatsync-chat and ~/.cache/heatsync-chat (delete those folders too if you want them gone)"
  fi
  exit 0
fi

here=$(cd "$(dirname "$0")" && pwd)
for f in heatsync-chat heatsync-chat.desktop icon-48.png icon-96.png; do
  [ -f "$here/$f" ] || { echo "$f is missing from $here: unpack the release again and run install.sh from inside that folder" >&2; exit 1; }
done

# the menu entry names the app by its full path. the desktop-entry spec wants anything past plain
# letters, digits and /._+- in double quotes with \ " ` $ backslashed, and % doubled
exec_path() {
  case "$1" in
    *[!A-Za-z0-9/._+-]*) printf '"%s"' "$(printf '%s' "$1" | sed 's/[\\"`$]/\\\\&/g; s/%/%%/g')" ;;
    *) printf '%s' "$1" ;;
  esac
}

# each file lands whole or not at all: written beside its place, then renamed into it (a running copy keeps running)
trap 'rm -f -- "$app.new" "$entry.new" "$icon48.new" "$icon96.new"' EXIT
put() { # mode source destination
  mkdir -p -- "$(dirname "$3")"
  install -m "$1" "$2" "$3.new"
  mv -f -- "$3.new" "$3"
}
put 755 "$here/heatsync-chat" "$app"
mkdir -p -- "$(dirname "$entry")"
line="Exec=$(exec_path "$app")" awk '/^Exec=/ { print ENVIRON["line"]; next } { print }' "$here/heatsync-chat.desktop" > "$entry.new"
chmod 644 "$entry.new"
mv -f -- "$entry.new" "$entry"
put 644 "$here/icon-48.png" "$icon48"
put 644 "$here/icon-96.png" "$icon96"

echo "done: open 'heatsync chat' from your app menu, or run $app"
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "(add $HOME/.local/bin to your PATH to run it as just: heatsync-chat)" ;; esac
echo "to remove it later: sh \"$here/install.sh\" --uninstall"
