#!/bin/sh
# puts heatsync chat in your home folder (no root): the app, its menu entry and icon.
# run it from the unpacked release folder:  ./install.sh
# the app updates itself from then on (the ↑ button on its tab bar)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
mkdir -p "$bin" "$apps" "$icons/48x48/apps" "$icons/96x96/apps"
install -m755 "$here/heatsync-chat" "$bin/heatsync-chat"
sed "s|^Exec=.*|Exec=$bin/heatsync-chat|" "$here/heatsync-chat.desktop" > "$apps/heatsync-chat.desktop"
install -m644 "$here/icon-48.png" "$icons/48x48/apps/heatsync-chat.png"
install -m644 "$here/icon-96.png" "$icons/96x96/apps/heatsync-chat.png"
echo "done: open 'heatsync chat' from your app menu, or run $bin/heatsync-chat"
case ":$PATH:" in *":$bin:"*) ;; *) echo "(add $bin to your PATH to run it as just: heatsync-chat)" ;; esac
