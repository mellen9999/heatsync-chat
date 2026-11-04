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
- **privacy invariant:** a word renders as a heatsync emote iff the SENDER's own inventory has it. catalog/search caches never feed the render path. one chokepoint: `src/emotes/resolve.rs` (`find_word` → `resolve`; its `Sources` are the only inputs it can see) — nothing else turns a word into an emote
- every server frame/response is hostile: bound sizes, validate urls against the image host allowlist (`docs/contract.md`)
- palette: ansi 0-7 + `#444444` zebra, `#808080` dim (`#bcbcbc` on a zebra stripe), `#ff8700` heatsync, `#c8a8ff` twitch, `#00ad03` ffz mod badge tile, `#5f0000`/`#870000` mention fill (ext's, plain/zebra row) — all in `src/ui/theme.rs`, nowhere else
- never a 🔥 on/next to a heat score
- tokens never rendered or logged (streamer-safe)

## budget (heatlaptop, dwl)
- anon rss ~12MB at start, ~21-28MB steady on a busy merged twitch+kick tab (chatterino: ~135MB anon). file-backed rss = mmapped fonts + binary, shared/evictable
- glibc parks freed heap after big json parses: `util::mem` caps arenas at 2, pins the mmap threshold at 128KB (else a +16MB fragmentation jump), and trims 2s after bursts. keep it
- append frame ~0.25ms, full redraw ~0.45ms; busy channel w/ animated emotes ~3% cpu (animations capped 30fps); idle cpu 0
- animated heatsync paints (scenes + effects, `src/ui/name_art/`): ~0.05ms per visible painted name per tick, 17 on screen ~2.4% cpu (`HSC_PAINTS=20 HSC_BENCH_H=520 --bench`), +4MB anon; ticks only for names on screen, 24-30fps, off with `animate paints`
- images: decoded at display height (28px × scale), frames idle 60s drop back to disk, 24MB hard cap with lru eviction, raw bytes in `~/.cache/heatsync-chat/img` (pruned to 256MB)

## conventions
- **settings**: one table, `src/model/schema.rs` (label, tip, category, section, slash command, side effect). the settings screen, the slash switches, the settings file and the presets all read it. a new switch = one `Settings` field + one row. keys: `src/model/keys.rs` feeds the `?` screen; keep it in step with the handlers in `src/app/keys.rs` (and the layer files beside it)
- packaging: `packaging/arch` = local source build, in its own folder (makepkg's `src/` would hit ours): `cd packaging/arch && paru -Bi .`. its pkgver is computed when makepkg reads the file and never written back, so the tree stays clean (no `pkgver()` fn: that rewrites the file). `packaging/aur` = the `heatsync-chat-bin` AUR package (the signed release), refreshed and published by `packaging/aur.sh` after `packaging/release.sh`. releases carry a v2 signature (`.sig2`) the app requires; keys in `src/net/update.rs` KEYS (spare key kept offline). `install.sh --uninstall` undoes install.sh
- rustfmt 2-space, lowercase ui copy, conventional commits, no ai fingerprints
