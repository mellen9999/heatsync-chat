# heatsync-chat

the heatsync browser extension's multichat overlay, as a standalone pure-rust window. **the ext (`~/projects/heatsync-extension`) is the spec** — not chatterino. roadmap + rationale: `docs/PLAN.md`. heatsync api/ws contract + hostile-input bounds: `docs/contract.md`.

## run / test
- `cargo build --release`, `cargo test --release`, `cargo clippy --release -- -D warnings`, `cargo fmt`
- hooks: `git config core.hooksPath .githooks` (gitleaks + pii + fmt + clippy + test)
- `--bench` headless gate: prints frame times + rss (anon vs file-backed). `HSC_DUMP=x.ppm` dumps the frame
- `--flood` windowed synthetic chat; `HSC_PROF=1` prints per-frame ms
- on dwl: `WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000`, screenshot with `grim -g "x,y wxh"`

## non-negotiables
- **no plugin system, no split panes.** one pane + ext-style tabs
- **cpu rendering only** (softbuffer + own blitter). no gpu context — that alone costs 30-80MB
- **privacy invariant:** a word renders as a heatsync emote iff the SENDER's own inventory has it. catalog/search caches never feed the render path. one chokepoint
- every server frame/response is hostile: bound sizes, validate urls against the image host allowlist (`docs/contract.md`)
- palette: ansi 0-7 + `#444444` zebra, `#808080` dim, `#ff8700` heatsync, `#c8a8ff` twitch — all in `src/ui/theme.rs`, nowhere else
- never a 🔥 on/next to a heat score
- tokens never rendered or logged (streamer-safe)

## budget (m1 gate, heatlaptop, dwl)
- anon rss ~11MB windowed (chatterino: ~135MB anon). file-backed rss is mmapped fonts + binary, shared/evictable
- append frame ~0.2ms render, ~1.2ms incl. present; full redraw ~0.35ms; idle cpu 0

## conventions
- rustfmt 2-space, lowercase ui copy, conventional commits, no ai fingerprints
