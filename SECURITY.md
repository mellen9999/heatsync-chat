# security

## reporting a vulnerability

email **mellen@heatsync.org** with:

- a clear description of the issue
- steps to reproduce
- potential impact
- your preferred contact for follow-up

please don't open a public GitHub issue for security bugs. we'll acknowledge within 48 hours and aim to ship a fix within 7 days for critical issues.

## supported versions

the latest release. fixes ship as a new release, and the in-app updater checks its signature before it installs one.

## worth reporting

- your heatsync login token showing up in a log, on screen, or sent anywhere but heatsync.org (the twitch chat token goes to twitch's chat server only)
- a way to make the updater install a build that isn't signed by the release keys
- a chat line, server frame or image that crashes or hangs the app, or gets it to read or write outside its own folders
- the app talking to a host that isn't listed in [docs/PRIVACY.md](docs/PRIVACY.md)

## scope

every server frame and response is treated as hostile: sizes are bounded and image urls are checked against an allowlist ([docs/contract.md](docs/contract.md)). what the app sends, and to whom, is in [docs/PRIVACY.md](docs/PRIVACY.md).
