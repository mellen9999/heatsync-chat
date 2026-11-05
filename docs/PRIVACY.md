# privacy

every host heatsync-chat talks to, what it gets, and why. no analytics, no telemetry, no trackers, no crash reports sent anywhere. this list is read off the code; if you find a host that isn't here, that's a bug.

## the short version

- twitch chat goes straight from your machine to twitch. kick and youtube chat come through heatsync.
- your heatsync login token goes to `heatsync.org` and nowhere else. the one other token in play is the twitch chat token heatsync hands you at login: it goes to twitch's chat server, so you can send.
- tokens are never written to a log or drawn on screen.
- every request says `heatsync-chat/<version>` as its user agent and, like any website visit, the other side sees your ip.
- chat lines are never saved to disk. what is saved is listed at the bottom.

## every host

| host | what for | what leaves your machine | when |
|---|---|---|---|
| `irc-ws.chat.twitch.tv` (wss) | twitch chat | the channels you open. logged out: a random guest name (`justinfan…`). logged in: your twitch name, the twitch chat token, and the lines you type | while the app runs |
| `gql.twitch.tv` | twitch's public web api, with twitch's own public web client id. no cookies, no login | badge lists (global, and each channel you open: its name); a person's name color + picture (their name, when you open their card); prediction banner (the channel's name, every 15s while one runs, slowly otherwise); the stream half of `/status` (channel name); pfps (names on screen, off by default) | as listed. `/predbanner` off stops the banner checks, `/avatars` is off until you turn it on |
| `static-cdn.jtvnw.net` | twitch emote, badge and picture files | the file request | when a line shows one |
| `heatsync.org` (https + wss) | the heatsync side of everything, see below | see below | while the app runs |
| `cdn.heatsync.org` | heatsync emote and upload pictures | the file request | when a line shows one |
| `7tv.io` | 7tv emote data | the global set; for each channel you open, that channel's twitch id; the sets again after a dropped connection; **your own twitch id, each time the app starts logged in or you log in**, to find your 7tv user id (so a paint or badge you change on 7tv shows on your own lines right away) | when you open a channel, and at login |
| `events.7tv.io` (wss) | live emote changes and paint/badge changes | one connection that subscribes by 7tv id to each open channel's emote set and broadcaster, and to you. 7tv can tell from that which channels you have open | only while a channel with a 7tv set is open |
| `api.7tv.app` | emote search for tab-complete and the picker | the word you typed | only when you search |
| `cdn.7tv.app` | emote, paint and badge pictures | the file request | when a line shows one |
| `api.betterttv.net`, `cdn.betterttv.net` | bttv emotes and badges | the global set; each open channel's twitch id; the badge list (nothing about you); the file requests | when you open a channel. `/bttvbadges` off hides the badges (the list is still read once at start) |
| `api.frankerfacez.com`, `cdn.frankerfacez.com` | ffz emotes and badges | the global set; each open channel's twitch id (its emotes, its mod/vip badge art); the badge list; the word you typed in an emote search; the file requests | when you open a channel or search |
| `fourtf.com` | chatterino badge art | the file request | when a line shows one |
| `kick.com` | a kick channel's subscriber badge art; a kick chatter's profile picture | the channel's name (`/api/v2/channels/<name>`), the same call for both. kick *chat* does not go here, it comes through heatsync | when you open a kick channel (badges); with pfps on (pictures, off by default) |
| `files.kick.com` | kick emote pictures | the file request | when a line shows one |
| `yt3.ggpht.com`, `*.googleusercontent.com` | youtube emoji and picture files a relayed line carries | the file request | when a line shows one |
| `pronoundb.org` | pronouns on a card or name popup | the twitch id of that person | when you open their card. once a day per person. off with `/pronouns` |
| `api.github.com`, `github.com` | new versions | asking for the latest release (github sees your ip and which version you run); the download only if you press update | 20s after start, then every 6 hours. off with `/updates` |

images are only ever fetched from the hosts above (an allowlist; anything else, or a link that sneaks credentials into the address, is refused). pictures from any other site that show under a chat line go through heatsync's picture proxy, so that site never sees you.

clicking a link or a video ▶ hands it to your own browser or `mpv`. those talk to that site themselves, outside this app.

## what heatsync sees

`heatsync.org` is first-party: same people as the app. it sees:

- **always:** your ip and the app version. the channels you have open (twitch, kick, youtube): the app joins each on heatsync's live connection and sends a heartbeat every 25s, and asks for each channel's recent lines, pin and linked accounts. the ids of the chatters whose lines are on your screen (to fetch their emote sets, paints and badges, and their sub flair), and the names of the twitch chatters on screen (to fetch their heat for the heat bar; `/heatbars` off stops it, `/subflair` off stops the flair asks). a name you open a card or hover popup for, and the channel you were in. a `#tag` you hover.
- **links and pictures in chat** (`/media` on, which is the default): the link of a line that previews (to resolve it) and the address of pictures from other sites (to proxy them). turn `/media` off and neither is sent.
- **things you search:** gif search words, chat-log searches, and which person's chat logs you open.
- **when you log in:** all of the above is tied to your account, and your token is sent with requests that need it: your synced settings (only the switches that sync, see `docs/contract.md`), follows, mutes and blocks, notes, whispers and dms, feed posts, uploads, sending to kick and youtube, twitch and kick moderation actions, pinning a line and `/nuke` (they run through heatsync, not from your machine), a twitch clip you ask for from a card (the channel's name), your emote inventory and changes you make to it, your notifications, who you follow that's live, your list of tabs (synced so your other heatsync windows open the same ones), and which tabs you have looked at.
- **uploads:** the file you pick. jpeg, png, webp and gif are stripped of gps, camera and phone model, timestamps, comments, xmp and thumbnails first (a phone photo keeps its rotation). avif, bmp, tiff and video go as they are, so strip those yourself if they carry a location.

it does **not** get your twitch chat lines from this app (they go straight to twitch), and a switch that is off sends nothing: `/media` for links and pictures, `/pronouns`, `/avatars`, `/updates`, `/predbanner` (opening the prediction view still asks), `/paints` + `/hspaints` for name paints and badges, `/heatbars` and `/subflair` for heat and flair.

a heatsync emote only draws on a line if that sender's own heatsync inventory has it, and never from a search or a catalog. that holds for live lines, kick and youtube lines, and archived chat logs.

## what stays on your machine

in `~/.config/heatsync-chat` (the folder is private to you, `0700`, and so is every file, `0600`; older installs are tightened at start):

- `settings`, `tabs`, `notes`, `filters`, `blocked` (emotes you hid), `hidden_paints`, `emote-frecency`, `recent`, `recent-gifs`
- `token`: your heatsync login
- `crash.log`: a line per crash, with anything secret cut out. it stays here; nothing sends it anywhere
- `<name>.bak`: a file the app couldn't read gets moved here and the chat says so, so nothing is lost silently

in `~/.cache/heatsync-chat/img` (also private): the pictures it downloaded, named by a sha256 of their address, capped at 256MB with the oldest dropped. safe to delete any time.

one window runs at a time (a lock in your runtime folder, `heatsync-chat.lock`); it holds nothing about you.
