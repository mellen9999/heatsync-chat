#!/usr/bin/env bash
# refresh the vendored vectors (the emote law, the tile marks, the modifiers, big emote-only lines, the emote notices, the filter rules,
# the synced settings catalog) from the site
# checkout, byte for byte. the files only ever change on the site side; never edit the copies here.
#   scripts/sync-emote-vectors.sh [path-to-site-checkout]
set -euo pipefail
site="${1:-$HOME/projects/heatsync}"
root="$(cd "$(dirname "$0")/.." && pwd)"
for pair in emote-law:src/law emote-mark:src/law emote-mods:src/emotes emote-only:src/emotes emote-notice:src/emotes filter-rules:src/model settings-sync:src/model; do
  src="$site/shared/${pair%%:*}.vectors.json"
  dst="$root/${pair##*:}/${pair%%:*}.vectors.json"
  [ -f "$src" ] || { echo "not found: $src" >&2; exit 1; }
  cp "$src" "$dst"
  sha256sum "$dst" | cut -d' ' -f1 > "${dst%.json}.sha256"
  echo "${pair%%:*} $(cat "${dst%.json}.sha256")"
done
echo "now run: cargo test --release law:: mods:: only:: say:: rules:: sync::"
