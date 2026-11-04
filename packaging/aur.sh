#!/bin/sh
# the AUR package heatsync-chat-bin: the signed github release as an arch package, no compiling.
#   sh packaging/aur.sh [vX.Y.Z]  refresh packaging/aur (PKGBUILD + .SRCINFO) for that release, default the
#                                 latest. downloads its files, checks them against the release's own
#                                 SHA256SUMS, writes the version and checksums. publishes nothing
#   sh packaging/aur.sh push      publish what is committed in packaging/aur to the AUR. needs your ssh key
#                                 registered on aur.archlinux.org. the only step that leaves this machine
# after a release:  sh packaging/release.sh, then sh packaging/aur.sh, look at the diff and commit it,
# then sh packaging/aur.sh push
set -eu
cd "$(git rev-parse --show-toplevel)"
dir=packaging/aur
repo=mellen9999/heatsync-chat
aur=ssh://aur@aur.archlinux.org/heatsync-chat-bin.git

fail() {
  echo "$1" >&2
  exit 1
}

refresh() {
  tag=${1:-$(gh release view --repo "$repo" --json tagName --jq .tagName)}
  ver=${tag#v}
  case "$tag" in v[0-9]*) ;; *) fail "that isn't a release tag: $tag" ;; esac
  case "$ver" in *[!0-9.]* | *..* | *.) fail "that isn't a release tag: $tag" ;; esac
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  base="https://github.com/$repo/releases/download/$tag"
  curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" || fail "can't fetch $tag's SHA256SUMS: is $tag out?"
  # the lines that go between the markers in the PKGBUILD: the arches this release has a build for, and their checksums
  arches=""
  sums=""
  for a in x86_64 aarch64; do
    f="heatsync-chat-$a-linux.tar.gz"
    want=$(awk -v f="$f" '$2 == f { print $1 }' "$tmp/SHA256SUMS")
    if [ -z "$want" ]; then
      echo "$tag has no $a build: leaving it out"
      continue
    fi
    curl -fsSL "$base/$f" -o "$tmp/$f" || fail "can't download $f"
    got=$(sha256sum "$tmp/$f" | cut -d' ' -f1)
    [ "$got" = "$want" ] || fail "$f doesn't match the release's SHA256SUMS: not touching anything"
    arches="$arches $a"
    sums="$sums
sha256sums_$a=('$got')"
  done
  [ -n "$arches" ] || fail "$tag has no build for any machine"
  old=$(sed -n 's/^pkgver=//p' "$dir/PKGBUILD")
  # a new version starts over at release 1; the same version again (a packaging fix) keeps its release number to bump by hand
  rel=$(sed -n 's/^pkgrel=//p' "$dir/PKGBUILD")
  [ "$old" = "$ver" ] || rel=1
  BLOCK="arch=(${arches# })$sums
" awk -v ver="$ver" -v rel="$rel" '
    /^pkgver=/ { print "pkgver=" ver; next }
    /^pkgrel=/ { print "pkgrel=" rel; next }
    /^# >>> / { print; printf "%s", ENVIRON["BLOCK"]; skip = 1; next }
    /^# <<</ { skip = 0 }
    !skip { print }
  ' "$dir/PKGBUILD" > "$tmp/PKGBUILD"
  cp "$tmp/PKGBUILD" "$dir/PKGBUILD"
  (cd "$dir" && makepkg --printsrcinfo) > "$tmp/SRCINFO"
  cp "$tmp/SRCINFO" "$dir/.SRCINFO"
  git --no-pager diff --stat -- "$dir"
  echo "done: $dir is ready for $tag. read the diff, commit it, then: sh packaging/aur.sh push"
}

push() {
  [ -z "$(git status --porcelain -- "$dir")" ] || fail "$dir has changes that aren't committed: commit them first, so the repo records exactly what the AUR gets"
  ver=$(sed -n 's/^pkgver=//p' "$dir/PKGBUILD")
  gh release view "v$ver" --repo "$repo" >/dev/null || fail "no release v$ver on github: nothing to publish"
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  (cd "$dir" && makepkg --printsrcinfo) > "$tmp/SRCINFO"
  cmp -s "$tmp/SRCINFO" "$dir/.SRCINFO" || fail ".SRCINFO doesn't match the PKGBUILD: run sh packaging/aur.sh, commit, then push"
  # the AUR keeps its own git history of just these two files, commits there are public under this name
  name=$(git config user.name || true)
  email=$(git config user.email || true)
  [ -n "$name" ] && [ -n "$email" ] || fail "set your git name and email first (git config user.name / user.email): AUR commits carry them"
  git clone -q "$aur" "$tmp/aur" || fail "can't reach the AUR: is your ssh key registered on aur.archlinux.org?"
  cp "$dir/PKGBUILD" "$dir/.SRCINFO" "$tmp/aur/"
  git -C "$tmp/aur" add PKGBUILD .SRCINFO
  if git -C "$tmp/aur" diff --cached --quiet; then
    echo "the AUR already has $ver"
    return 0
  fi
  git -C "$tmp/aur" --no-pager diff --cached --stat
  git -C "$tmp/aur" -c "user.name=$name" -c "user.email=$email" commit -q -m "heatsync-chat-bin $ver"
  echo "publishing $ver to the AUR as $name"
  git -C "$tmp/aur" push origin HEAD:master
  echo "done: heatsync-chat-bin $ver is on the AUR"
}

case "${1:-}" in
  push) [ "$#" -eq 1 ] || fail "usage: sh packaging/aur.sh [vX.Y.Z | push]"; push ;;
  "" | v*) [ "$#" -le 1 ] || fail "usage: sh packaging/aur.sh [vX.Y.Z | push]"; refresh "${1:-}" ;;
  *) fail "usage: sh packaging/aur.sh [vX.Y.Z | push]" ;;
esac
