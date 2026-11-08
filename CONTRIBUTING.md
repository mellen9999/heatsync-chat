# contributing

heatsync-chat is twitch, kick and youtube chat in one native window, with heatsync emotes. pure rust, drawn on the cpu, small on purpose.

## build and test

needs a current stable rust.

```sh
cargo build --release
cargo test --release
cargo fmt
cargo clippy --release --all-targets -- -D warnings
```

`./target/release/heatsync-chat --bench` draws without a window and prints frame times and memory: run it if you touch drawing.

`git config core.hooksPath .githooks` turns on the pre-commit checks (secrets scan, fmt, clippy, tests). the secrets scan needs `gitleaks`, and the commit is blocked without it.

## house rules

- rustfmt is set in `rustfmt.toml`: just run `cargo fmt`
- colours live in `src/ui/theme.rs` and nowhere else: ansi 0-7 plus a few brand tokens. don't add more
- the biggest source files have a line ceiling in `tests/loc-ceilings.txt`. raise one only with a reason, in the same commit
- a word only draws as someone's heatsync emote if that sender owns it. search and catalog results never feed the chat render path

## commits and pull requests

- conventional commits, lowercase, small and atomic: `fix(pick): esc lets a picked line go`
- one change per pull request. a behaviour change comes with a test
- run the four commands above before you open it

## licence

MIT. by contributing you agree your work is released under the repo's [LICENSE](LICENSE).

## security

found a vulnerability? don't open a public issue. see [SECURITY.md](SECURITY.md).
