# heatsync-chat

twitch, kick and youtube chat in one native window, with [heatsync](https://heatsync.org) emotes, paints and badges. it's the heatsync browser extension's multichat as its own app: same tabs, same keys, same look, no browser. pure rust, drawn on the cpu, around 25MB of its own memory on a busy channel, zero cpu when chat is quiet.

linux (wayland or x11). MIT.

## install

**from a release** (x86_64, no root needed):

1. download `heatsync-chat-x86_64-linux.tar.gz` from the [latest release](https://github.com/mellen9999/heatsync-chat/releases/latest)
2. in the folder you downloaded it to, run:

```sh
tar xf heatsync-chat-x86_64-linux.tar.gz && ./heatsync-chat/install.sh
```

it lands in `~/.local/bin` with a menu entry and icon. to take it out again: `./heatsync-chat/install.sh --uninstall`

**on arch, from this repo** (builds the committed code, adds the menu entry and icon):

```sh
git clone https://github.com/mellen9999/heatsync-chat && cd heatsync-chat/packaging/arch && paru -Bi .
```

**updates:** when a new version is out, a yellow `↑` shows on the tab bar. one click downloads it, checks its signature and restarts. `check for updates` in settings turns that off.

## use it

```sh
heatsync-chat                        # your tabs from last time
heatsync-chat forsen kick:xqc        # these channels, one tab each (just for this run)
heatsync-chat "forsen kick:forsen"   # quoted together: one tab with both chats merged
```

- `ctrl-t` adds a channel (twitch name, `kick:name`, `yt:@handle`), `ctrl-l` logs in with heatsync
- `?` shows every key, `ctrl-,` opens settings (or click ⚙)
- `ctrl-f` searches the tab, `tab` completes emotes and names (7tv, bttv and ffz too: `nyan` or `:nyan`)
- `/vi` turns on vim keys (`j`/`k`, `gg`/`G`, `gt`/`gT`)
- `ctrl-b` shows who you follow that's live, `ctrl-p` your heatsync emotes (rename, reorder, remove, share)
- `ctrl-n` picks which feeds a merged tab sends to (all of them by default)
- `/sharelayout` (or `/sl`) copies your tabs as a `heatsync.org/m/…` link, `/layout <link>` opens someone else's

## what's in it

- twitch, kick and youtube in one tab or many, plus feed, whispers, mentions and notifs tabs. a merged tab sends one line to all its feeds at once
- heatsync emotes: a word shows as someone's heatsync emote only if *they* own it. 7tv, bttv, ffz and native emotes, overlay (zero-width) emotes, animated
- heatsync paints, badges and name colors, live. bttv badges, sub flair and heat bars
- emote picker, gif search, picture and video upload (pictures lose their hidden location and camera data first)
- replies, pins, predictions, polls, subs, raids, cheers and redeems, mod actions on twitch and kick, mention highlights + desktop notification + sound
- went-live lines (+ a desktop notification), and who you follow that's live
- the message box says when you're timed out or banned and for how long, and marks text past the limit red
- links open in your browser, video links in mpv if you have it
- settings, presets and keys match the extension's

## optional extras

nothing here is required; each one turns on one thing.

| install | for |
|---|---|
| `wl-clipboard` (wayland) or `xclip` (x11) | copy and paste |
| `libnotify` | a desktop notification when you're mentioned |
| `xdg-utils` | opening the login page and links |
| `zenity` | the file picker for uploads |
| `mpv` + `yt-dlp` | video links play in their own window |

on arch: `paru -S wl-clipboard libnotify xdg-utils zenity mpv yt-dlp`

## privacy

no analytics, no telemetry, no crash reports sent anywhere. your login token goes to heatsync.org only and is never logged or drawn on screen. chat is never saved to disk. [docs/PRIVACY.md](docs/PRIVACY.md) lists every host the app talks to and what it sends.

your settings and tabs live in `~/.config/heatsync-chat`, the picture cache in `~/.cache/heatsync-chat`.

## build it

needs a current stable rust (`paru -S rust`, or [rustup](https://rustup.rs) elsewhere):

```sh
cargo build --release
./target/release/heatsync-chat
```

- `cargo test --release`: the test suite
- `--bench`: draws without a window and prints frame times and memory
- `--flood`: a window of fake busy chat
- `git config core.hooksPath .githooks`: the pre-commit checks (secrets scan, fmt, clippy, tests)

[docs/PLAN.md](docs/PLAN.md) explains how it's built and why, [docs/contract.md](docs/contract.md) has the heatsync api it speaks.
