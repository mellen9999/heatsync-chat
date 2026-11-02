#!/bin/sh
# cut a release from a clean, landed tree: test, build, sign, pack, publish to github.
#   bump `version` in Cargo.toml first, land it, then:  sh packaging/release.sh
# the signing key (ed25519) lives at ~/.config/heatsync-chat-release/sign.pem and never in the repo;
# the app only installs updates signed by it (src/net/update.rs KEY)
set -eu
cd "$(git rev-parse --show-toplevel)"
key="$HOME/.config/heatsync-chat-release/sign.pem"
[ -f "$key" ] || { echo "no signing key at $key"; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "tree is dirty: commit or land first"; exit 1; }
v=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
tag="v$v"
git ls-remote --exit-code --tags origin "$tag" >/dev/null 2>&1 && { echo "$tag is already out: bump the version"; exit 1; }
name="heatsync-chat-$(uname -m)-linux"
# linked against glibc 2.31 (zig as the linker) so it runs on distros years older than the build box
# (debian 11, ubuntu 20.04 and up). needs: paru -S cargo-zigbuild zig
target="$(uname -m)-unknown-linux-gnu"
command -v cargo-zigbuild >/dev/null || { echo "needs cargo-zigbuild: paru -S cargo-zigbuild"; exit 1; }
cargo test --release
cargo zigbuild --release --target "$target.2.31"
built="target/$target/release/heatsync-chat"
need=$(objdump -T "$built" | grep -o 'GLIBC_[0-9.]*' | sort -V | tail -1)
echo "needs $need"
out=$(mktemp -d)
cp "$built" "$out/$name"
openssl pkeyutl -sign -rawin -inkey "$key" -in "$out/$name" -out "$out/$name.sig"
mkdir "$out/heatsync-chat"
cp "$built" packaging/install.sh packaging/heatsync-chat.desktop LICENSE "$out/heatsync-chat/"
cp assets/icon-48.png assets/icon-96.png "$out/heatsync-chat/"
tar -C "$out" -czf "$out/$name.tar.gz" heatsync-chat
(cd "$out" && sha256sum "$name" "$name.tar.gz" > SHA256SUMS)
gh release create "$tag" --target "$(git rev-parse HEAD)" --title "$tag" \
  --notes "install (linux $(uname -m)): download $name.tar.gz, then
\`\`\`
tar xf $name.tar.gz && ./heatsync-chat/install.sh
\`\`\`
already installed? the ↑ button on the tab bar updates in one click." \
  "$out/$name.tar.gz" "$out/$name" "$out/$name.sig" "$out/SHA256SUMS"
rm -rf "$out"
echo "released $tag"
