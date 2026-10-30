# heatsync-chat

the heatsync multichat as a native app — twitch + kick + youtube in one window, heatsync emotes everywhere, ~10MB of ram.

```sh
cargo build --release
./target/release/heatsync-chat            # open
./target/release/heatsync-chat --flood    # synthetic chat flood (render test)
./target/release/heatsync-chat --bench    # headless render timing + memory
```

keys: `j`/`k` scroll · `ctrl-d`/`ctrl-u` half page · `G` newest
