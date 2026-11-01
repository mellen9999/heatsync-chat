# privacy

every host heatsync-chat talks to, and why. first-party only: no analytics, no telemetry, no trackers. tokens are never logged or rendered.

- twitch chat (irc over websocket): the channels you open
- `gql.twitch.tv` (anonymous, twitch's public web client id, no cookies): badge images, and a linked twitch account's own name color. the color read sends only that account's login, once per person, cached; a 3s limit, a 4MB cap, and the answer must be a plain `#rgb`/`#rrggbb` or it is ignored
- heatsync api + ws: profiles and cards for names you hover or that are named in chat, cosmetics, the feed and whispers when logged in
- 7tv, bttv, ffz: emote sets and paints by id
- kick, youtube: only for tabs that include them
