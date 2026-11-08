# privacy

every host heatsync-chat talks to, what it gets, and why. no analytics, no telemetry, no trackers, no crash reports sent anywhere. this list is read off the code; if you find a host that isn't here, that's a bug.

## the short version

- twitch chat goes straight from your machine to twitch. kick and youtube chat come through heatsync.
- your heatsync login token goes to `heatsync.org` and nowhere else. the one other token in play is the twitch chat token heatsync hands you at login: it goes to twitch's chat server, so you can send.
- tokens are never written to a log or drawn on screen.
- every request says `heatsync-chat/<version>` as its user agent and, like any website visit, the other side sees your ip.
- chat lines are never saved to disk, bar the lines that pinged you (the mentions tab, see the bottom) and, only if you turn on `save chat logs` (`/chatlogs`, off by default), every line of your open channels. what is saved is listed at the bottom.

## every host

| host | what for | what leaves your machine | when |
|---|---|---|---|
| `irc-ws.chat.twitch.tv` (wss) | twitch chat | the channels you open. logged out: a random guest name (`justinfan…`). logged in: your twitch name, the twitch chat token, and the lines you type | while the app runs |
| `gql.twitch.tv` | twitch's public web api, with twitch's own public web client id. no cookies, no login | badge lists (global, and each channel you open: its name); a person's name color + picture (their name, when you open their card); prediction banner (the channel's name, every 15s while one runs, slowly otherwise); the stream half of `/status` (channel name); whether a channel is live and since when, once as its history loads, so a stream already under way gets its went-live line in the history (channel name; `/online` off stops it); pfps (names on screen, off by default) | as listed. `/predbanner` off stops the banner checks, `/avatars` is off until you turn it on |
| `static-cdn.jtvnw.net` | twitch emote, badge and picture files | the file request | when a line shows one |
| `heatsync.org` (https + wss) | the heatsync side of everything, see below | see below | while the app runs |
| `cdn.heatsync.org` | heatsync emote and upload pictures | the file request | when a line shows one |
| `7tv.io` | 7tv emote data | the global set; for each channel you open, that channel's twitch id; the sets again after a dropped connection; **your own twitch id, each time the app starts logged in or you log in, but only while `7tv paints + badges` is on** (`/paints`; off sends nothing), to find your 7tv user id (so a paint or badge you change on 7tv shows on your own lines right away). with `7tv personal emotes` on (`/personalemotes`), also the id of a 7tv emote set (never a person's id) each time a chatter's personal set is first met | when you open a channel, at login, and when a chatter with a personal set speaks |
| `events.7tv.io` (wss) | live emote changes and paint/badge changes | one connection that subscribes by 7tv id to each open channel's emote set and broadcaster, and to you (only while `7tv paints + badges` is on). with `7tv personal emotes` on it also subscribes to each open twitch channel's twitch id (to hear which chatters there have a personal set) and to each such set's id. nothing about you goes with it: your own twitch id is not in any of these. 7tv can tell from all of this which channels you have open | only while a channel with a 7tv set is open, or `7tv personal emotes` is on with a twitch channel open |
| `api.7tv.app` | emote search for tab-complete and the picker | the word you typed (in the picker: what is in the filter box, once you stop typing for a moment, and again for more pages; the `7tv` chip off stops it) | only when you search |
| `cdn.7tv.app` | emote, paint and badge pictures | the file request | when a line shows one |
| `api.betterttv.net`, `cdn.betterttv.net` | bttv emotes and badges | the global set; each open channel's twitch id; the badge list (nothing about you); the file requests | when you open a channel. `/bttvbadges` off hides the badges (the list is still read once at start) |
| `api.frankerfacez.com`, `cdn.frankerfacez.com` | ffz emotes and badges | the global set; each open channel's twitch id (its emotes, its mod/vip badge art); the badge list; the word you typed in an emote search (tab-complete, or the picker's filter box after a short pause; the `ffz` chip off stops the picker's); the file requests | when you open a channel or search |
| `fourtf.com` | chatterino badge art | the file request | when a line shows one |
| `kick.com` | a kick channel's subscriber badge art; a kick chatter's profile picture | the channel's name (`/api/v2/channels/<name>`), the same call for both. kick *chat* does not go here, it comes through heatsync | when you open a kick channel (badges); with pfps on (pictures, off by default) |
| `files.kick.com` | kick emote pictures | the file request | when a line shows one |
| `yt3.ggpht.com`, `*.googleusercontent.com` | youtube emoji and picture files a relayed line carries | the file request | when a line shows one |
| `pronoundb.org` | pronouns on a card or name popup | the twitch id of that person | when you open their card. once a day per person. off with `/pronouns` |
| `api.github.com`, `github.com`, `*.githubusercontent.com` | new versions | asking for the latest release (github sees your ip and which version you run); the download only if you press update (github sends it from its own file host, `*.githubusercontent.com`) | 20s after start, then every 6 hours. off with `/updates` |

images are only ever fetched from the hosts above (an allowlist; anything else, or a link that sneaks credentials into the address, is refused). pictures from any other site that show under a chat line go through heatsync's picture proxy, so that site never sees you.

clicking a link or a video ▶ hands it to your own browser or `mpv`. those talk to that site themselves, outside this app.

## what heatsync sees

`heatsync.org` is first-party: same people as the app. it sees:

- **always:** your ip and the app version. the channels you have open (twitch, kick, youtube): the app joins each on heatsync's live connection and sends a heartbeat every 25s, and asks for each channel's recent lines, pin and linked accounts. every 90s (and just after you open a tab) it asks which of your open twitch and kick channels are live, for the red dot on a live tab (their names, no login). the ids of the chatters whose lines are on your screen (to fetch their emote sets, paints and badges, and their sub flair), and the names of the twitch chatters on screen (to fetch their heat for the heat bar; `/heatbars` off stops it, `/subflair` off stops the flair asks). a name you open a card or hover popup for, and the channel you were in. a `#tag` you hover.
- **links and pictures in chat** (`/media` on, which is the default): the link of a line that previews (to resolve it) and the address of pictures from other sites (to proxy them). turn `/media` off, or just the preview cards with `/linkpreviews`, and the link is not sent.
- **things you search:** gif search words, chat-log searches, and which person's chat logs you open. the emote picker's filter box too: the word in it (2 to 50 letters, digits and `_-:.`, nothing else is sent) goes to heatsync's emote directory a moment after you stop typing, with no login and no token. the `hs` chip off stops it. what comes back are thumbnails in the picker, nothing more.
- **when you log in:** all of the above is tied to your account, and your token is sent with requests that need it: your synced settings (only the switches that sync, see `docs/contract.md`), follows, mutes and blocks, notes, whispers and dms, feed posts, uploads, sending to kick and youtube, twitch and kick moderation actions, pinning a line and `/nuke` (they run through heatsync, not from your machine), a twitch clip you ask for from a card (the channel's name), your emote inventory and changes you make to it, your notifications, who you follow that's live (asked every 2 minutes while you're logged in, so a kick or youtube go-live gets its went-live line), your twitch follow list when you open `/import` (heatsync asks twitch for it with your link; heatsync sees that you asked, and the live check that sorts it sends the names of those channels, 20 at a time, to heatsync without your token), your list of tabs (synced so your other heatsync windows open the same ones), and which tabs you have looked at.
- **uploads:** the file you pick. jpeg, png, webp and gif are stripped of gps, camera and phone model, timestamps, comments, xmp and thumbnails first (a phone photo keeps its rotation). avif, tiff and video (mp4, mov, webm) are cleaned the same way or refused; only bmp goes as it is, and it can't carry that data.

it does **not** get your twitch chat lines from this app (they go straight to twitch), and a switch that is off sends nothing: `/media` for links and pictures (`/linkpreviews` for just the preview cards), `/pronouns`, `/avatars`, `/updates`, `/predbanner` (opening the prediction view still asks), `/paints` + `/hspaints` for name paints and badges, `/personalemotes` for 7tv personal emotes, `/heatbars` and `/subflair` for heat and flair.

a heatsync emote only draws on a line if that sender's own heatsync inventory has it, and never from a search or a catalog (the picker's catalog tiles are previews that vanish when it closes; a picked one joins your inventory only when you send a line with it). that holds for live lines, kick and youtube lines, and archived chat logs.

a 7tv personal emote (the set a 7tv subscriber picked, which 7tv shows in any chat) draws only on lines that same chatter sent, and only when 7tv itself says that chatter's twitch id owns the set: never from a search, a catalog or a guess, never on a kick or youtube line, and never offered to you in tab-complete or the picker unless it is your own set. your own set is found from the same 7tv reply that finds your 7tv user, so it needs `7tv paints + badges` on too; `7tv personal emotes` adds nothing about you to what goes to 7tv. at most 1000 chatters' sets are held, about 4000 emotes in all, in memory only: nothing is written to disk.

## what stays on your machine

in `~/.config/heatsync-chat` (the folder is private to you, `0700`, and so is every file, `0600`; older installs are tightened at start):

- `settings`, `tabs`, `notes`, `filters`, `blocked` (emotes you hid), `hidden_paints`, `emote-frecency`, `recent`, `recent-gifs`, `tabsync` (your heatsync account's tab list as of the last sync, with your heatsync name, so tabs can be matched back after a restart; used only for that account), `presets` (only from older versions, which had settings presets; nothing reads it now and it is safe to delete), `picker-search` (which picker chips are on; never synced)
- `mentions`: the last 500 lines that pinged you, with who sent them and which channel, tagged with your heatsync name so only you get them back. the mentions tab uses it to survive a restart; a file over a day old is dropped, and logging out (or another account logging in) deletes it
- `token`: your heatsync login
- `crash.log`: a line per crash, with anything secret cut out. it stays here; nothing sends it anywhere
- `<name>.bak`: a file the app couldn't read gets moved here and the chat says so, so nothing is lost silently

in `~/.local/share/heatsync-chat/logs` (`$XDG_DATA_HOME` if you set it; private, `0700`/`0600`), **only with `save chat logs` on, which is off until you turn it on**: `<platform>/<channel>/<yyyy-mm-dd>.log`, one plain-text line per message (`[hh:mm:ss] name: text`), plus timeouts, bans, subs and other notices. it covers the lines that arrive while the app runs and the channels you have open; history that loads when you open a tab is not saved. whispers and dms are never saved. it never leaves your machine, and the app never reads, trims or deletes it: the disk space is yours to manage. turning the switch off stops writing; the files stay until you delete them. if a write fails (disk full, no permission) the chat says so once and logging stops until you flip the switch off and on.

in `~/.cache/heatsync-chat/img` (also private): the pictures it downloaded, named by a sha256 of their address, capped at 256MB with the oldest dropped. safe to delete any time.

one window runs at a time (a lock in your runtime folder, `heatsync-chat.lock`); it holds nothing about you.
