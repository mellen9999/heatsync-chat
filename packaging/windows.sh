#!/bin/sh
# run a cargo command for windows (x86_64-pc-windows-gnu) from linux, with zig as the C compiler and the linker:
#   sh packaging/windows.sh build --release                                  the exe
#   sh packaging/windows.sh clippy --release --all-targets -- -D warnings    lint it
#   sh packaging/windows.sh test --release -- --test-threads=1               run the tests under wine (needs wine)
# the exe lands in $CARGO_TARGET_DIR (default target/) x86_64-pc-windows-gnu/release/heatsync-chat.exe.
# needs: zig, and the rust target (rustup target add x86_64-pc-windows-gnu). with cargo-zigbuild installed
# (paru -S cargo-zigbuild) `build` goes through it; without it the same thing is done here with three tiny
# wrapper scripts, written to a temp folder and deleted after
set -eu
cd "$(git rev-parse --show-toplevel)"
target=x86_64-pc-windows-gnu
[ "$#" -gt 0 ] || { echo "usage: sh packaging/windows.sh <cargo command and flags>"; exit 2; }
command -v zig >/dev/null || { echo "needs zig: paru -S zig"; exit 1; }
[ -d "$(rustc --print target-libdir --target "$target")" ] || { echo "needs the rust target: rustup target add $target"; exit 1; }
cmd=$1
shift

if [ "$cmd" = build ] && command -v cargo-zigbuild >/dev/null; then
  exec cargo zigbuild --target "$target" "$@"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# zig cc / ar / dlltool under the names cargo and rustc look for. the cc wrapper drops what zig spells its own way
# (the target flags rustc and the cc crate add), the static/dynamic switches and the mingw runtime libs it brings
# itself, and maps rustc's -lgcc_eh to zig's -lunwind
cat > "$tmp/zcc" <<'W'
#!/bin/bash
out=()
skip=0
for a in "$@"; do
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  case "$a" in
    --target=*) continue;;
    -target) skip=1; continue;;
    -Wl,-Bstatic|-Wl,-Bdynamic) continue;;
    -lgcc_eh) out+=(-lunwind); continue;;
    -lgcc|-lmingw32|-lmingwex|-lmsvcrt|-l:libpthread.a) continue;;
    -Wl,--high-entropy-va|-Wl,--disable-auto-image-base|-Wl,-O1) continue;;
  esac
  out+=("$a")
done
exec zig cc -target x86_64-windows-gnu "${out[@]}"
W
printf '#!/bin/sh\nexec zig ar "$@"\n' > "$tmp/zar"
printf '#!/bin/sh\nexec zig dlltool "$@"\n' > "$tmp/dlltool"
chmod +x "$tmp/zcc" "$tmp/zar" "$tmp/dlltool"

export CC_x86_64_pc_windows_gnu="$tmp/zcc"
export AR_x86_64_pc_windows_gnu="$tmp/zar"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$tmp/zcc"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C dlltool=$tmp/dlltool"
cargo "$cmd" --target "$target" "$@"
