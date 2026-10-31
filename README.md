# heatsync-chat

the heatsync multichat as a native app: twitch + kick + youtube in one window, heatsync emotes everywhere, ~10MB of ram.

run it from source:

```sh
cargo build --release
./target/release/heatsync-chat            # open (no args = your last tabs)
./target/release/heatsync-chat forsen xqc # open these channels
```

install it on arch (builds from the committed git history, adds the menu entry and icon):

```sh
cd packaging/arch && paru -Bi .
```

other flags: `--flood` (synthetic chat, render test), `--bench` (headless render timing + memory)

keys: `?` lists every key, `ctrl-,` opens settings (or click the ⚙), `/` or `ctrl-f` searches the tab, `ctrl-l` logs in, `ctrl-t` adds a channel. vi users: `/vi` turns on `j`/`k`, `gg`/`G`, `gt`/`gT`
