#!/bin/sh
# cut a release from a clean, landed tree: test, build, sign, pack, publish to github.
#   bump `version` in Cargo.toml first, land it, then:  sh packaging/release.sh
# the signing key (ed25519) lives at ~/.config/heatsync-chat-release/sign.pem and never in the repo;
# the app only installs updates signed by it or by the spare key kept offline (src/net/update.rs KEYS).
# every build ships two signatures:
#   <asset>.sig2  what the app checks: covers the version, the asset name and the binary's sha256
#   <asset>.sig   the binary bytes alone: only copies installed before 0.3.0 read it (x86_64 only; drop it
#                 once nobody runs those)
# when it's out and looks right, the arch package follows:  sh packaging/aur.sh   (see that file)
set -eu
cd "$(git rev-parse --show-toplevel)"
key="$HOME/.config/heatsync-chat-release/sign.pem"
[ -f "$key" ] || { echo "no signing key at $key"; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "tree is dirty: commit or land first"; exit 1; }
[ "$(uname -m)" = x86_64 ] || { echo "release from an x86_64 machine (it cross-builds the rest)"; exit 1; }
v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
tag="v$v"
git ls-remote --exit-code --tags origin "$tag" >/dev/null 2>&1 && { echo "$tag is already out: bump the version"; exit 1; }

# a key the app doesn't trust would publish a release nobody can install: check before building anything.
# the app's keys are the 32-byte arrays in src/net/update.rs, the signing key's public half is the last 32 bytes of its DER
pub=$(openssl pkey -in "$key" -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n')
trusted=$(sed -n '/^const KEYS/,/^];/p' src/net/update.rs | grep -o '0x[0-9a-f][0-9a-f]' | cut -c3- | tr -d '\n' | fold -w64)
printf '%s\n' "$trusted" | grep -qx "$pub" || { echo "the signing key isn't one src/net/update.rs trusts: not releasing"; exit 1; }

# linked against glibc 2.31 (zig as the linker) so it runs on distros years older than the build box
# (debian 11, ubuntu 20.04 and up). needs: paru -S cargo-zigbuild zig rust-aarch64-gnu
command -v cargo-zigbuild >/dev/null || { echo "needs cargo-zigbuild: paru -S cargo-zigbuild zig"; exit 1; }
arches="x86_64 aarch64"
for arch in $arches; do
  [ -d "$(rustc --print target-libdir --target "$arch-unknown-linux-gnu")" ] || { echo "needs the $arch rust target: paru -S rust-$arch-gnu"; exit 1; }
done

cargo test --release
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

# ed25519 over what src/net/update.rs signed_message builds: header, version, asset name, sha256 of the binary
sign2() { # bin asset sig
  msg="$out/msg"
  printf 'heatsync-chat update v2\n%s\n%s\n%s\n' "$v" "$2" "$(sha256sum "$1" | cut -d' ' -f1)" > "$msg"
  openssl pkeyutl -sign -rawin -inkey "$key" -in "$msg" -out "$3"
  openssl pkeyutl -verify -rawin -pubin -inkey "$out/pub.pem" -in "$msg" -sigfile "$3" >/dev/null
}
openssl pkey -in "$key" -pubout -out "$out/pub.pem"

files=""
sums=""
for arch in $arches; do
  name="heatsync-chat-$arch-linux"
  target="$arch-unknown-linux-gnu"
  # (the aarch64 one finds libfontconfig when it starts, not at link time: see Cargo.toml)
  cargo zigbuild --release --target "$target.2.31"
  built="target/$target/release/heatsync-chat"
  need=$(readelf --version-info "$built" | grep -o 'GLIBC_[0-9.]*' | sort -V | tail -1)
  echo "$arch needs $need"
  [ "$(printf '%s\nGLIBC_2.31\n' "$need" | sort -V | tail -1)" = GLIBC_2.31 ] || { echo "$arch build needs more than glibc 2.31"; exit 1; }
  cp "$built" "$out/$name"
  sign2 "$out/$name" "$name" "$out/$name.sig2"
  if [ "$arch" = x86_64 ]; then
    openssl pkeyutl -sign -rawin -inkey "$key" -in "$out/$name" -out "$out/$name.sig"
    files="$files $name.sig"
  fi
  pack="$out/pack-$arch/heatsync-chat"
  mkdir -p "$pack"
  cp "$built" packaging/install.sh packaging/heatsync-chat.desktop LICENSE "$pack/"
  cp assets/icon-48.png assets/icon-96.png "$pack/"
  tar -C "$out/pack-$arch" -czf "$out/$name.tar.gz" heatsync-chat
  files="$files $name $name.sig2 $name.tar.gz"
  sums="$sums $name $name.tar.gz"
done
# shellcheck disable=SC2086 # file names this script made, no spaces
(cd "$out" && sha256sum $sums > SHA256SUMS)

upload=""
for f in $files SHA256SUMS; do upload="$upload $out/$f"; done
# shellcheck disable=SC2086
gh release create "$tag" --target "$(git rev-parse HEAD)" --title "$tag" \
  --notes "install (linux): download the .tar.gz for your machine, then
\`\`\`
tar xf heatsync-chat-x86_64-linux.tar.gz && ./heatsync-chat/install.sh
tar xf heatsync-chat-aarch64-linux.tar.gz && ./heatsync-chat/install.sh
\`\`\`
(the first line for x86_64 machines, the second for aarch64)
already installed? the ↑ button on the tab bar updates in one click." \
  $upload
echo "released $tag"
echo "next: sh packaging/aur.sh   (refreshes the arch package for it; pushing it is a separate step)"
