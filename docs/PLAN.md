# heatsync-chat — the ext's multichat, as a native rust app

## context
- chatterino: ~243MB RSS on heatlaptop; lua plugin is capped by what chatterino exposes (no transforms, no paints, no settings ui)
- goal: the ext's multichat overlay as a standalone window in pure rust, every feature that can exist outside a browser, **<30MB RSS, ~0% idle cpu**
- scope set by mellen: ext is the spec. NO plugin system, NO splits — one pane + ext tabs (live/feed/whispers/mentions/pinned + channels)
- precedent: foot (cpu-rendered, damage-tracked, ~15MB) — same approach, same look

## repo
- `~/projects/heatsync-chat`, public `mellen9999/heatsync-chat`, MIT, gitleaks + pii + fmt + clippy + tests via `.githooks`, rustfmt 2-space, clippy `-D warnings`, no CI
- release profile: lto=fat, codegen-units=1, strip. `packaging/arch` (source build, `paru -Bi .`), `packaging/aur` (`heatsync-chat-bin`), `packaging/release.sh` (signed release, x86_64 + aarch64), `packaging/install.sh` (no-root install, `--uninstall`)
- docs: `README.md` (what + install + use), `docs/contract.md` (heatsync api/ws contract + hostile-input bounds), `docs/PRIVACY.md` (every host contacted + what leaves the machine)

## stack (each dep earns its weight)
| layer | choice | why |
|---|---|---|
| window/input/ime | winit 0.30 | wayland(dwl)+x11+windows, ime, hidpi |
| pixels | softbuffer (wl_shm) + own blitter | no gpu ctx (mesa/vk alone = 30-80MB); ui is rects+glyphs+images only |
| text | parley (inline boxes for emotes/badges, bidi, fallback via fontique) + swash raster → glyph cache | correct cjk/emoji/rtl, system monospace |
| net | tokio current_thread on ONE bg thread; tokio-rustls, tokio-tungstenite, reqwest(rustls, gzip) | epoll = zero idle wakeups, real timeouts |
| images | image-webp, gif, png (pure rust) | decode at display height only |
| misc | serde/serde_json | settings and wire json |
- notifications/sound/links/clipboard: spawn `notify-send` / `pw-play` / `xdg-open` / `wl-copy` + `wl-paste` or `xclip` (no dbus/audio/clipboard deps), best-effort, logged once if missing
- no ring/openssl-free compromise needed: rustls default provider; no C++/qt/webview anywhere

## architecture
```
src/
  main.rs      boot, args, winit loop, ui⇄net channels (EventLoopProxy wake)
  app.rs       the App: tabs, settings, wiring. app/ holds one file per concern
               (keys, pointer, render, layout, input, ime, net_events, tabs, complete, picker,
               overlay = the one floating menu slot, card, feed, whispers, mentions, mods, pred, pin, update…)
  ui/          canvas (own blitter), text (parley + glyph cache), chat_view/ (virtualized rows:
               layout, draw, hit), tab_bar, input_bar, picker, card, menu, form, sheet,
               name_art/ (animated paints), theme (ansi 0-7 + the few named extras, nowhere else)
  model/       message, chan + ident (channel/login rules), schema (THE settings table),
               keys (feeds the ? screen), slash, filters/automod/rules, feed, whisper, pred, layout_link…
  emotes/      resolve.rs (THE render chokepoint: privacy invariant), providers (7tv/bttv/ffz/twitch/kick),
               live (7tv EventAPI), block
  heatsync/    senders (each chatter's own set + ownership window), late_refs, content gates
  cosmetics/   heatsync + 7tv paints, badges, name fx
  badges/      twitch/ffz/bttv/chatterino badges
  images/      decode (webp/gif/png at display height), cache (byte-capped lru + disk), url allowlist
  net/         one tokio current_thread: irc (twitch), hs_ws (heatsync /ws), seventv, session, http,
               relay (kick/yt), send_queue, emote_ops, upload, update (signed self-update), clip…
  store/       ~/.config/heatsync-chat, atomic writes, 0700/0600, single-instance lock
  util/        clip (wl-copy/xclip), crash (redacted crash.log), mem (glibc arena caps), open, sound, time
```
- one render chokepoint: `emotes/resolve.rs` — word renders iff SENDER's inventory has it; catalog/search cache never feeds render
- render loop: winit `ControlFlow::Wait`; redraw only on change; new messages = memmove framebuffer + draw new rows; animated emotes = damage only their rects at their frame times; pause when unfocused/occluded
- memory: messages ~1000/channel ring (setting `hs_dom_render_cap` → scrollback cap), layout lazily for visible rows only, frame cache byte cap (default 24MB), off-screen emotes evicted to disk-cached bytes
- streamer-safe: tokens/emails never rendered or logged (redacted), device codes only on screen

## data sources (from research)
- **twitch chat**: irc over `wss://irc-ws.chat.twitch.tv` (tags+commands); anon justinfan read, authed via heatsync `GET /api/auth/twitch/chat-token` (same as ext `auth-irc.js`); USERNOTICE = subs/gifts/raids/announcements banners
- **kick + youtube**: relayed by heatsync `/ws` (`kick-chat-message`, `youtube:chat/status`, backfill) — no direct kick/yt connections; kick send `POST /api/kick/chat/send`; youtube send `POST /api/youtube/chat/send` (official data api, brokered; ctrl-l → allow youtube posting)
- **heatsync ws** `wss://heatsync.org/ws?b=1`: join/leave, emote:watch, ack:channels, youtube:subscribe, heartbeat 25s, `authenticate {token}` when logged in; inbound stream:*, pin:*, moment:spike, heat updates, emote:*, mutes/seen-state/settings
- **emotes**: 7tv v3 rest + EventAPI ws (live set updates, paints, badges), bttv, ffz, twitch, chatterino badges via `/api/chatterino-badges`, recent-messages via `/api/recent-messages/{ch}`
- **actions via server**: whispers `/api/twitch/whisper` + EventSub `user.whisper.message` read, mod `/api/mod/*`, clips `/api/twitch/clip`, mutes `/api/mutes`, blocks `/api/user/blocks`, settings `/api/settings`, seen-state `/api/user/seen-state`, feed `/api/messages*`, card `/api/card`
- full endpoint catalog: `heatsync/server/lib/api-catalog/*.ts`; ws types: `server/services/ws-protocol.ts`

## server addition (heatsync repo, own `cw` worktree, `/cpd`)
- native login = loopback + PKCE: `GET /api/auth/login?native=1&redirect=http://127.0.0.1:<port>/cb&code_challenge=…` → `POST /api/auth/native-exchange` returns `{token, exp, refresh}`; cookieless `POST /api/auth/refresh`; bearer logout
- touch points: `lib/oauth-url-policy.ts` (allow 127.0.0.1 loopback), `routes/auth-routes.ts` (reuse mobile code flow at :1218/:2954), `/api/native/health` mirroring `/api/extension/health` (min version + killswitch)
- one login = heatsync + twitch irc token + kick send + all server-brokered actions. pilotable: "log in" button → browser opens → done

## skipped (named, not hidden)
- lost natively (need twitch web-session integrity tokens): prediction/poll voting, channel-point redeems + auto-claim, follow/unfollow, highlighted messages, resub share — read-only banners for predictions/polls only if eventsub allows, else skipped
- browser-only: overlay positioning, player volume, title flash, noise-hide css, yt layout toggles
- i18n 35 langs: english first, string table structured for later (ext strings reusable)

## milestones (each ships complete + verified)
1. **skeleton + render gate** — window, text w/ fallback, scroll-blit, fake 10k-msg flood. GATE: RSS <20MB, flood redraw <2ms/frame on heatlaptop; if parley/softbuffer misses, swap before building on it
2. **twitch read** — anon irc, channel tabs, twitch/7tv/bttv/ffz emotes (animated, zero-width, provider priority), badges, recent-messages backfill, timestamps, zebra, readable names, platform badges, first-chatter glow, cleared msgs, paints, 7tv EventAPI
3. **heatsync read (anon)** — inventory by login, sender sets + privacy invariant, modifiers (real transforms), cw gate, 🔥 marker, heat bars (6 tiers, never 🔥 beside score), flair, name colors, pin, live/spike lines, kick+yt relay w/ [K][Y], backfill, community mutes, link previews/inline images via `/api/img`
4. **login + send** — server addition, native login, irc send, kick send, reply threading, input (history 50, vi-mode port of `chrome/vi-mode.js`, wysiwyg emote preview), tab-complete (ext `compareAcMatches` order), emote picker (virtual grid, recents, catalog search), gifs, mentions (tint/sound/notif, mention-rules)
5. **tabs + social** — live, feed (posts, like, reply, composer, trending, heat tiers), whispers (eventsub + send), mentions, pinned; seen-state + settings sync; user card (+ pronouns, logs, clips); archive search
6. **mod + filters** — mod toolbar/keys (delete/timeouts/ban/unban/purge, confirm, reasons), client automod (caps + regex w/ redos guard), filter-rules dsl, hide bots/commands/dupes/keywords, mute (timed, kick/yt→twitch alias, server sync), block, emote block
7. **settings + polish** — schema-driven settings view (port native-relevant keys of `settings-schema.js`, presets incl low-ram), keyboard map overlay, PKGBUILD, 24h soak

## ui rules (mellen design taste)
- monospace system font 15px, dark only, ansi 0-7 palette, square everything, hover/active = white bg black text, emphasis = bold/underline/reverse/blink only, unicode icons, lowercase copy, no motion
- keyboard-first: `j/k` `gg/G` `ctrl-d/u` scroll, `/` filter (`@name` scope), `i` insert, esc normal, `gt/gT` + `alt-1..9` tabs, `:` command line (ext slash commands), `?` keymap overlay; mouse fully works too

## verification
- `cargo test`: irc parser, heatsync ws frames (fuzz every type w/ hostile shapes, like plugin harness), resolve.rs invariant ("lost" must not render from catalog), modifiers grammar, tab-complete order, heat tiers, lookup cache swr/negative ttl, ws backoff/generation/watchdog
- headless render snapshots: cpu renderer draws to a buffer → golden png compare (no display needed)
- real run on dwl: join a busy channel, `grim` screenshot → read it; RSS via `/proc/<pid>/status` vs chatterino's 243MB; `perf stat`/`pidstat` idle + flood cpu
- each milestone: the ext side-by-side in chrome on the same channel — same emotes, same markers, same lines
- 24h soak: RSS flat (if glibc creep shows, `malloc_trim` after evictions)
