# heatsync-chat → windows: port plan (read-only study, repo untouched)

base: origin/main @ c4753e7, v0.3.2. scratch: /tmp/claude-1000/wp (copy of the tree, build output, logs).

## 0. the short version
- the app is already 90% portable: winit, softbuffer, parley/swash, tokio, reqwest+rustls(ring), image decoders, tiny-skia all build for windows. **no dep blocks the port.**
- real `cargo check --target x86_64-pc-windows-gnu`: **36 errors**, all in the unix glue (list in §3). they are resolution errors, so a second wave of type errors may hide behind them (expect a handful).
- the work is ~14 small replacements of "run a unix tool / use a unix api" with a win32 call via `windows-sys`.
- recommended build path: **x86_64-pc-windows-gnu cross-built with zig** (zig is already installed and the release script already uses cargo-zigbuild). not msvc/xwin.
- recommended ship: one bare signed `.exe` asset + `.sig2` (same updater, same signature), plus a zip for first download. no installer.

## 1. inventory (file:line → what → windows equivalent)

### compile blockers (the 36 errors)
| where | what | windows |
|---|---|---|
| src/app.rs:10-11,748-749 | `WindowAttributesExtWayland/X11::with_name` (app id / WM_CLASS) | `#[cfg(unix)]` them out. window icon (`with_window_icon`, app.rs:744) already works on windows; taskbar icon needs the exe icon resource (see §5, `winresource`-free: embed .ico by a tiny build.rs or skip v1) |
| src/app/raise.rs:8,13 | `ActiveEventLoopExtWayland::is_wayland` (close+reopen window to get focus) | cfg(unix) only. windows branch = the existing else arm (`set_minimized(false)` + `focus_window()`); windows blocks focus-steal, so also `request_user_attention` flashes the taskbar button (already there). fine |
| src/store/mod.rs:14,134-148,241,252 | `DirBuilderExt/OpenOptionsExt/PermissionsExt`: 0700 dirs, 0600 files, repair of loose modes | windows: files under `%APPDATA%` are already private to the user by the profile ACL. cfg-out the mode calls and the repair loop (no-op). tests that assert modes → `#[cfg(unix)]`. optionally keep `create_new` semantics (not unix-only) |
| src/store/mod.rs:22-34 | `xdg("XDG_CONFIG_HOME", ".config")`, `XDG_DATA_HOME` (logs dir) | config → `%APPDATA%\heatsync-chat` (roaming; settings+tabs are small and should follow the user), logs → `%LOCALAPPDATA%\heatsync-chat\logs`. read via `std::env::var_os("APPDATA")` — no api call needed. reject non-absolute |
| src/images/cache.rs:5,10-13,44-48,92,114 | `XDG_CACHE_HOME`, 0700/0600 | `%LOCALAPPDATA%\heatsync-chat\cache` (not roaming — 256MB cache). cfg-out modes |
| src/chatlog.rs:15,179-184,200 | 0700/0600 + `O_NOFOLLOW` (`libc::O_NOFOLLOW`, no symlink follow) | windows: plain create. symlinks need admin on windows so the attack is much narrower; add a `metadata().is_symlink()` refusal before open to keep the intent. `FILE_FLAG_OPEN_REPARSE_POINT` via `custom_flags` (std `OpenOptionsExt` windows) is the exact analog if wanted |
| src/store/lock.rs:5-6,34-41 | `flock(LOCK_EX|LOCK_NB)` single instance, lock file in `XDG_RUNTIME_DIR` | **named mutex**: `CreateMutexW(null, 1, L"Local\\heatsync-chat")`, `GetLastError()==ERROR_ALREADY_EXISTS` → `Some(Err(()))`; handle kept in `Held` (released by the os at exit, same "no stale lock" property). `Local\` prefix = per-session, so two logged-in users don't collide. `sibling()` (also used by wake) → `%LOCALAPPDATA%\heatsync-chat` or unneeded on windows |
| src/store/wake.rs:6-7,60-90,140-176 | unix socket 0600 beside the lock; "show\nopen x\n", 1s deadline, 2KB cap, "ok\n" ack | **named pipe** `\\.\pipe\heatsync-chat-<sid>`: server `CreateNamedPipeW` (PIPE_REJECT_REMOTE_CLIENTS, default DACL = owner + admins; pass an explicit SECURITY_ATTRIBUTES only if needed), client `CreateFileW`/`std::fs::OpenOptions` on the pipe path. keep `encode/decode/clean` + caps as they are (pure code, already tested). std has no pipe server → ~120 lines of windows-sys. a blocking thread in `ConnectNamedPipe` = idle cpu 0. read deadline: overlapped read or `CancelIoEx` from a timer; simplest: a watchdog thread that calls `CancelSynchronousIo` after 1s. alternative that needs **no pipe**: **loopback tcp on 127.0.0.1:random + port written to a file** — rejected: any local process/user can talk to it, and a firewall prompt may appear. use the pipe |
| src/util/time.rs:32-35 | `libc::localtime_r` / `libc::tm` (local clock for chat timestamps, log day) | `SystemTimeToTzSpecificLocalTime` (windows-sys `Win32_System_Time`) after `FileTimeToSystemTime`; or `GetTimeZoneInformation` bias math. 25 lines, keep `Local{y,mo,d,h,mi,s}` |
| src/util/open.rs:19-22 | `installed()` via exec bit on PATH (also used for pw-play/mpv) | windows: `PATH` + `PATHEXT` search (`.exe`), no exec bit. keep `installed(bin)` signature |
| src/net/update.rs:187-195,211-215 | write tmp 0755, `rename` over running exe, then `exec()` into new copy | see §5 — windows can't overwrite a running exe but CAN rename it. needs rewrite of `replace` + `restart` |

### runtime features that compile but would not work
| where | what | windows |
|---|---|---|
| src/util/clip.rs (whole, Backend::Wayland/X11, `Tool` args) + src/util/spawn.rs + src/net/clip.rs | clipboard via wl-copy/wl-paste/xclip, mime picking, size cap, deadline | win32 clipboard: `OpenClipboard` (retry 3x, windows holds it for others), `GetClipboardData(CF_UNICODETEXT)`, `EnumClipboardFormats` for offered types, images: registered format `"PNG"` (browsers, snipping tool) else `CF_DIBV5`/`CF_DIB` → needs a bmp→rgba step (we already decode png; a dib header strip + our own tiny 32-bit bmp reader ≈ 60 lines; or wrap the DIB in a BMP file header and hand to... no bmp decoder in deps → write it). set text: `SetClipboardData(CF_UNICODETEXT, GlobalAlloc'd utf16)`. keep the `Fail` enum and the "blocking thread + NetEvent::Pasted" shape (net/clip.rs) unchanged. `Backend::of/pkg` + their tests stay unix. windows needs no external package so `Fail::Missing(pkg)` never fires |
| src/util/notify.rs:40 | `notify-send` | cheapest honest option: a **tray balloon / toast without com**: `Shell_NotifyIconW` with `NIF_INFO` balloon (windows 10/11 shows it as a normal toast). needs a hidden message window + tray icon for the app's lifetime (small, but it's a tray icon the owner may not want) — alternative: **powershell toast = shell, forbidden**. alternative 3: `windows` crate WinRT `ToastNotification` — pulls a heavy crate + needs an AppUserModelID shortcut. **pick Shell_NotifyIconW, add the icon on first notification, delete it 10s after the last** (no permanent tray icon). reuse `clean()` (strip markup escapes not needed for balloon: skip `&amp;` escaping on windows — `clean` must take a flag) and caps (balloon title ≤63 utf16, body ≤255) |
| src/util/sound.rs:18,115-135 | pw-play/paplay/aplay fed a wav on stdin | `PlaySoundW(wav_ptr, null, SND_MEMORY|SND_ASYNC|SND_NODEFAULT)` from `windows-sys Win32_Media_Audio` (winmm). **zero extra deps, no process, the synthesised wav bytes are already built in memory** (`synth()`). note: SND_MEMORY|SND_ASYNC requires the buffer to stay alive until done → keep it in a `static Mutex<Vec<u8>>` and replace it for the next sound, or play `SND_SYNC` on the existing spawned thread (sound.rs already plays off-thread; sync on that thread is simplest). links `winmm` |
| src/util/open.rs:72 + src/net/auth.rs:244 | `xdg-open` for links and for the login browser | `ShellExecuteW(null, "open", url, null, null, SW_SHOWNORMAL)` (windows-sys `Win32_UI_Shell`). **no shell involved**; the url is already validated by `check()` (http/https, no spaces/control, ≤2048). `ShellExecute` on a validated https url opens the default browser. dedupe: auth.rs should call `util::open::open_url` (one place) |
| src/util/open.rs:44-58 | `mpv` launch (`--no-config --load-scripts=no --force-window=immediate --no-terminal --`) | works as is if `mpv.exe` is on PATH (`installed("mpv")` with PATHEXT). `Command::new("mpv")` on windows also searches the exe's dir and cwd (cwd search = planted-exe risk; use the `installed()` resolved absolute path, not the bare name). exit codes 1/2 same. add `CREATE_NO_WINDOW` (`creation_flags(0x08000000)`) so no console flashes. streamlink is **not used anywhere** in the code (grep: no hit) — only mpv |
| src/util/font.rs + src/net/font.rs + app/font.rs | `/font <name>` resolved with `fc-match` (family, file) | windows: fontique already has a **DirectWrite backend** (`dwrite.rs`, default `system` feature), `Monospace` generic = **Consolas** (dwrite.rs:34). so default fonts work with no work. `/font name`: resolve with parley's `FontContext.collection.family_id(name)` directly (it is already the check used in `use_family`, ui/text.rs:116) — on windows skip fc-match and the file/mmap step: `Found.file` becomes optional. the "mmapped fonts" budget line: dwrite loads system fonts through the OS font cache, file-backed pages are shared; fine. emoji: `GenericFamily::Emoji` → Segoe UI Emoji via dwrite (colour glyphs: check swash renders colr/ cbdt; segoe is COLR v0/1 → verify in phase 2) |
| src/net/upload.rs:269-290 | file picker via zenity / kdialog; `parse_pick` requires `/`-absolute path | windows: `GetOpenFileNameW` (comdlg32, windows-sys `Win32_UI_Controls_Dialogs`), blocks like zenity does (already off-thread). `parse_pick` → accept `C:\…` / `\\server\…` absolute paths: `Path::is_absolute()` instead of `starts_with('/')`. also `expand()`: `~/`, `file://` handling for windows paths |
| src/app/upload.rs:16, src/util/wav.rs:129,363 | `HOME` env must start with `/` (for `~/` expansion) | windows: `USERPROFILE`; `is_absolute()` |
| src/app/streamer.rs:30, src/model/streamer.rs:16-29 | obs detection by reading `/proc/<pid>/comm == "obs"` every 10s; `root` is injectable for tests | windows: `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)` + `Process32FirstW/NextW`, match `szExeFile` == `obs64.exe` (also `obs32.exe`; streamlabs = `Streamlabs OBS.exe`? out of scope, mirror linux: exact obs only). cap `SCAN_MAX`. keep `obs_running(root)` for unix + tests; add `obs_running_win()`; State.root becomes a cfg field |
| src/util/mem.rs | `mallopt/malloc_trim` (already `cfg(all(linux,gnu))` with empty fallback) | **already done: no-op on windows.** but the *reason* (glibc parking freed heap) has a windows twin: the win32 heap (HeapAlloc) returns big blocks to the os decently, but fragmentation after json parse bursts could differ. measure in phase 3; if a jump shows, `HeapCompact`/`SetProcessWorkingSetSize(-1,-1)` after bursts = the `trim()` analog. `libc` dep must become `cfg(unix)` so it isn't dragged to windows |
| src/bench.rs:10 | rss from `/proc/self/status` (anon vs file-backed) | windows: `GetProcessMemoryInfo` (psapi) → `PrivateUsage` (≈ anon) and `WorkingSetSize`. anon-vs-file split doesn't exist; report private + working set. the budget numbers (12/21-28MB anon) need new windows baselines, not the linux ones |
| src/util/crash.rs | panic hook → crash.log beside settings | works (uses store::append). cfg only mode bits. consider `windows_subsystem` stderr loss — crash.log is the only place windows users get a message (good: it exists already) |
| src/main.rs:20-36,60+ | `--help`, `-V`, second-launch message to stdout, `eprintln!` everywhere | **release exe must be `#![windows_subsystem = "windows"]`** (no console window flashing next to the gui). then stdout/stderr go nowhere. fix: at start, `AttachConsole(ATTACH_PARENT_PROCESS)` so `--help` from a terminal prints; and messages that matter to a double-clicking user must be a **message box** (`MessageBoxW`) or a notify — "already open", "can't open the window". keep `eprintln` for dev |
| src/main.rs:33, schema.rs:587, other text | help text and tips hardcode `~/.config/heatsync-chat`, `~/.local/share/heatsync-chat/logs` | cfg'd constants for the folder text (a small `store::where_text()`) so windows says `%APPDATA%\heatsync-chat` |
| packaging/install.sh, heatsync-chat.desktop, aur, arch | linux packaging | not used on windows. windows "install" = unzip anywhere, run (updater swaps in place). optional later: `install.ps1` (**powershell is a shell but it's a script the user runs, not the app**; still, owner rule is about the app, so allowed) — v1: none |
| signals | grep `signal|SIGTERM|ctrlc`: **none in the code** (no handler). windows: closing the window = process exit via winit; ctrl-c from a console just kills. nothing to port |
| `Command::new` inventory (complete) | notify-send (notify.rs:40), mpv (open.rs:47), xdg-open (open.rs:72, auth.rs:244), fc-match (via spawn.rs:64, font.rs:65), wl-*/xclip (via spawn.rs:64), zenity/kdialog (upload.rs:271), pw-play/paplay/aplay (sound.rs:130), `mkfifo` (net/mod.rs:1421, **test only**, `#[cfg(unix)]` it), `cat`/`yes` (spawn.rs tests, unix-only) | all covered above. after the port windows spawns **only mpv** |
| tests (unix-only) | mode asserts in store/mod.rs, images/cache.rs, chatlog.rs, update.rs:392-401, tests_mentions.rs:76, wake.rs:173; `/tmp/..` paths in upload.rs:374-401; `cat`/`yes` in spawn.rs; `mkfifo` | gate with `#[cfg(unix)]` or give windows variants (named mutex/pipe tests replace lock/wake tests) |

### things that look unix but are fine
- `cfg(target_arch = "aarch64")` fontique/fontconfig-dlopen (Cargo.toml:49): irrelevant on windows.
- `softbuffer` + winit: win32 backend is native, no gpu context (GDI `BitBlt`/`StretchDIBits` into the window). matches the "cpu only" rule.
- `ring`: needs a C compiler for its asm; works with zig cc (verified: it compiled in the scratch check).
- keyboard: vim keys via winit logical keys; IME (`app/ime.rs`) uses winit `Ime` events — works on windows, verify.
- `std::env::current_exe` fine. `/usr` check in `self_updates()` (update.rs:124): windows has no package manager copy; replace with "folder is writable" and "not under Program Files unless writable" (the probe-write already covers it).

## 2. dependency check (Cargo.lock)
build for windows: winit 0.30, softbuffer 0.4, parley 0.12 + fontique 0.12 (dwrite backend via `windows`+`windows-core` crates — compile-time weight only, no runtime cost beyond dwrite.dll), swash, icu_segmenter, tokio (rt/net/time/sync), tokio-tungstenite + rustls(ring) + webpki-roots, reqwest 0.13 (rustls-no-provider; **uses webpki roots, no schannel/native-roots needed**, `schannel` in the lock is only a reqwest optional/`native-tls` leftover — not built), image-webp/gif/png, tiny-skia, serde. **all compile on windows-gnu** (the check reached the project crate with zero dep failures once a C compiler for ring existed).
- `libc`: unix glue only → make it `[target.'cfg(unix)'.dependencies]`.
- wayland-*, x11rb/x11-dl, smithay-client-toolkit, rustix: winit/softbuffer pull them on linux only; absent on windows.
- **to add:** `windows-sys` (already in the lock at 0.52/0.59/0.61 via other crates; pick 0.61 to dedupe). features only: `Win32_Foundation`, `Win32_System_Threading` (mutex), `Win32_System_Pipes`, `Win32_Storage_FileSystem`, `Win32_Security`, `Win32_System_DataExchange`, `Win32_System_Memory` (GlobalAlloc), `Win32_System_Ole` (CF_ consts), `Win32_System_Diagnostics_ToolHelp`, `Win32_System_ProcessStatus`, `Win32_System_Time`, `Win32_Media_Audio` (PlaySound), `Win32_UI_Shell` (ShellExecuteW, Shell_NotifyIconW), `Win32_UI_WindowsAndMessaging` (message box, tray hidden window), `Win32_UI_Controls_Dialogs` (open file), `Win32_System_Console` (AttachConsole). windows-sys is raw bindings: **no runtime cost, ~0 binary bloat**, only what's referenced is linked. gate the whole dep `[target.'cfg(windows)'.dependencies]`.
- nothing else. no `windows` crate in our code, no `arboard`/`notify-rust`/`rodio`/`rfd` (each would cost 0.5-3MB and a tree of deps).

## 3. real compile errors (cross check, x86_64-pc-windows-gnu)
```
src/store/mod.rs 6    src/store/lock.rs 6    src/net/update.rs 5   src/images/cache.rs 5
src/chatlog.rs 4      src/store/wake.rs 3    src/util/open.rs 2    src/app.rs 2
src/app/raise.rs 2    src/util/time.rs 1                          total 36
```
all are `std::os::unix` imports / `libc::flock|localtime_r|O_NOFOLLOW` / `winit::platform::{wayland,x11}`. nothing in net/, ui/, model/, emotes/, images decoders, heatsync/ — **the rendering and protocol code compiles untouched**. logs: /tmp/claude-1000/wp/check3.txt. caveat: after these are fixed, rustc will type-check the remaining bodies; expect a few more (e.g. `Command` creation flags, `PathBuf` `/` assumptions in `expand`).

## 4. build path from this arch box
- **recommend: `x86_64-pc-windows-gnu` via `cargo zigbuild`.** zig (installed, /usr/bin/zig) is the C compiler and linker, exactly what release.sh already does for glibc 2.31 and aarch64. rustup target already added in this session (`rustup target add x86_64-pc-windows-gnu`, user-level; harmless). no mingw-w64 needed (`x86_64-w64-mingw32-gcc` is absent; the plain check failed there, with zig as CC it passed ring). no windows sdk license issues.
- msvc via `cargo-xwin`: not installed; downloads the ms CRT/sdk (~1GB, microsoft license) at build time; slightly smaller/faster binaries, better debug info, and the "right" abi if we ever ship a crate that needs msvc. not worth it for a pure-rust, no-c++ app.
- the gnu build statically links its runtime (`-C target-feature=+crt-static` is the default for -gnu with zig): **no libgcc/libwinpthread dlls to ship**, single exe. verify in phase 1 with `x86_64-w64-mingw32-objdump -p`/`zig objdump` that imports are only system dlls (kernel32, user32, gdi32, ws2_32, bcrypt, dwrite, winmm, shell32, comdlg32, ole32, psapi…).
- icon + version resource in the exe: a tiny `build.rs` compiling a `.rc` needs windres (absent) → either embed a prebuilt `.res`/`.o` (committed ~10KB) or skip the exe icon in v1 (window icon via winit still works at runtime; only explorer/taskbar-pinned icon is generic). **recommend: v1 skip, phase 4 add.**
- cross-built ring asm with zig cc: confirmed to compile. link check still to do (phase 1).
- try-it result so far: `rustup target list --installed` was `x86_64-unknown-linux-gnu` only; added the windows-gnu target; check = 36 errors above.

## 5. release / updater
current: asset `heatsync-chat-<arch>-linux` (+`.sig2`, + `.tar.gz`); app picks the asset via `asset_name()` = `heatsync-chat-{ARCH}-linux` (update.rs:68), verifies ed25519 over `heatsync-chat update v2\n<version>\n<asset>\n<sha256>\n`, renames over the exe, `exec()`s.

**windows asset:** `heatsync-chat-x86_64-windows.exe` and `heatsync-chat-x86_64-windows.exe.sig2`.
- `asset_name()` → `heatsync-chat-{ARCH}-windows.exe` under `cfg(windows)`. the signed message already binds the asset name, so a linux binary can't be replayed onto windows. **signature scheme unchanged**; `parse()` URL allowlist unchanged; release.sh `sign2` already generic.
- release.sh: add after the linux loop: `cargo zigbuild --release --target x86_64-pc-windows-gnu`, copy to `$out/heatsync-chat-x86_64-windows.exe`, `sign2`, and for first download a **zip** (`heatsync-chat-x86_64-windows.zip` containing the exe + LICENSE + a 5-line `README.txt`). add to `files`/`sums`. run the readelf glibc check only for linux. skip `.sig` (legacy). `cargo test --release` in release.sh runs linux tests only; add a wine smoke (below) or a manual gate.
- **replacing a running exe on windows:** you can't write over it, but you **can rename it**. update flow: write `heatsync-chat.new.exe` (flushed; `FILE_FLAG_WRITE_THROUGH`/`sync_all`) → `rename(exe → heatsync-chat.old.exe)` → `rename(new → exe)` (if the second fails, rename old back: atomic-enough, never leaves the app missing) → on restart: spawn the new exe (`Command::new(path).args(..).spawn()`, then `process::exit(0)` — no exec() on windows) and at the next start delete `*.old.exe` (it's unlocked once the old process is gone). antivirus can briefly lock the new file → retry the rename 5×200ms and say a plain failure ("couldn't swap the app in: close other copies and try again") otherwise. `replace_failed` text stays plain-lowercase.
- the single-instance mutex: the new process starts while the old one still holds it → the restart must release first (drop `Held`) or the child retries the mutex for ~3s before concluding "already open". use the retry (bulletproof, no handoff protocol).
- `self_updates()`: on windows true if the exe folder is writable (the probe already does it). a copy in `C:\Program Files` isn't writable → button hidden, same as `/usr` today.
- no installer in v1: unzip, run. rationale: minimal; an msi/inno/nsis adds a toolchain, uninstall registry entries, admin prompts, and the updater already self-updates. add start-menu shortcut + uninstall later only if asked.
- **code signing:** unsigned v1 (matches the approved plan in memory). smartscreen warns on first run of any unsigned exe until reputation builds (hundreds of downloads), a signed (OV/EV) cert later (~$200-400/yr, EV instant reputation) or Azure Trusted Signing (~$10/mo, needs a verifiable business/individual identity). the app's own ed25519 `.sig2` still protects the updater regardless; smartscreen only hits the first manual download (the updater's `rename`d exe downloaded by our own process has no mark-of-the-web, so **updates don't warn**).

### smartscreen walkthrough text (download page, 3 steps)
```
windows may say "windows protected your pc". that's only because the app is new and not signed yet.
1. click "more info" (the small blue link in that box)
2. click "run anyway"
3. that's it. it won't ask again, and the app updates itself from then on
(want proof? the file's sha256 is on the release page next to the download)
```
also note: if the zip was downloaded, "right-click → properties → unblock" is the alternative. keep it to the 3 steps above on the page.

## 6. testing
- this box has `/usr/bin/wine` (wine-11.21; no `wine64` shim, `wine` itself is 64-bit-capable; `~/.wine` prefix not created yet — first run creates it, a few seconds, uses ~500MB disk. not run, per instructions).
- what wine CAN prove: the exe starts, `--version`, `--help`, `--bench` (headless; software render so it works; but rss numbers meaningless), named mutex + named pipe (wine implements both), clipboard text, `PlaySound` no error, config paths (`%APPDATA%` → `~/.wine/drive_c/users/…`). run unit tests under wine: `cargo test --target x86_64-pc-windows-gnu --no-run` then `wine target/.../deps/heatsync_chat-*.exe` with `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine` — cheap CI-like gate for the pure-logic tests (store, wake codec, updater swap on a temp exe).
- what wine can NOT prove: real window latency, softbuffer/GDI redraw cost, DirectWrite font fallbacks and colour emoji, dpi scaling across monitors, SmartScreen, real toasts, mpv/browser launching, memory/cpu budget, antivirus locking during update.
- unit tests under the windows target: gate unix-mode tests `#[cfg(unix)]`; add windows tests: mutex second-taker, pipe wake roundtrip + hostile input (oversize/slow/garbage — port the existing wake tests 1:1), `replace` swaps while the old exe file is "running" (hold a handle open), path resolution with `APPDATA` unset → fails loud, `parse_pick` with `C:\`, `time::Local::of` against `std::time` known day.
- **manual checklist for mellen's windows machine** (heatpc dual-boots windows — so no VM needed: boot windows on heatpc; heatpc is 32GB so also not representative of the low-ram budget — also try on a weak/older machine if there is one):
  1. smartscreen walkthrough works as written
  2. window opens, correct size, taskbar icon, title, dark titlebar? (winit `with_theme(Dark)`)
  3. text sharp at 100/125/150% dpi; move window between monitors with different dpi
  4. vim keys, ctrl-t, `/` commands, ime typing (win+. emoji picker, a CJK ime)
  5. connect twitch + kick + yt tabs, emotes (static, animated, 7tv/heatsync paints) render; cpu ≈ idle 0 when nothing happens (task manager), animated channel ≈ 3%
  6. private working set vs the linux ~21-28MB anon; check after a 30-min busy chat (leak/fragmentation)
  7. second launch (double-click again, and `heatsync-chat.exe somechannel` from cmd) → raises the first, opens the channel
  8. copy chat text out / paste text and a screenshot (win+shift+s) in / upload picker
  9. mention → toast appears, sound plays, click-through not needed; 1.2s pacing
  10. link click → default browser; `w` → mpv if on PATH; login flow opens browser + returns
  11. obs auto streamer-mode: start/stop obs64.exe, within 10s
  12. kill the process mid-save (task manager) → relaunch: settings/tabs intact (atomic writes)
  13. update button: install old build, publish new, click ↑ → app restarts on new version, `.old.exe` gone on next run; tampered download refused with the plain message
  14. no console window ever appears; `--help` works from cmd
  15. fonts: default consolas, `/font cascadia mono` works, a missing font says so
  16. antivirus/defender doesn't quarantine (note: unsigned rust exes sometimes false-positive; keep builds reproducible-ish, submit to defender if flagged)

## 7. phased plan, sizes, risks
| phase | work | size |
|---|---|---|
| **1 compile** | `cfg(unix)` the 36 sites; split each unix module into `unix.rs`/`windows.rs` behind one function surface (store perms, lock, wake, time, open/installed, update replace, streamer, bench rss); Cargo target-specific deps (`libc` unix-only, `windows-sys` windows-only); stubs returning "not on windows yet" so it links; link with zigbuild; `windows_subsystem` + AttachConsole; run `--version` under wine | **M** |
| **2 run** | real impls of the core: paths (`APPDATA`/`LOCALAPPDATA`), mutex, pipe wake, local time, ShellExecute (links + login), mpv via PATHEXT, fonts check (Consolas, emoji), window icon, `raise` | **M** |
| **3 parity** | clipboard (text then image), notifications (balloon), sound (PlaySound), file picker, obs detection, bench rss, font command via dwrite, text strings with windows paths, mem measurement + `trim` analog if needed, settings tips | **L** (clipboard images + the notification tray lifecycle are the long poles) |
| **4 ship** | updater windows swap + restart + `.old` cleanup, `asset_name`, release.sh windows branch + zip, README/landing link ("small + OS-aware" per memory), exe icon resource, smartscreen text, wine gate, manual checklist on heatpc-windows, 0.3.3→ first windows release | **M** |
total ≈ 1 L + 3 M. phases 1-2 can land independently (linux unaffected, tests green on both).

### risks
1. **colour emoji / font fallback on dwrite** (segoe ui emoji COLR rendering through swash) — unknown until run; mitigation: the heatsync/twitch emotes are images, only unicode emoji affected; worst case monochrome fallback.
2. **memory/cpu budget** (12→28MB anon) was tuned for glibc; the win32 heap, DirectWrite font cache and softbuffer's DIB section behave differently → new baselines, maybe a trim analog.
3. **named-pipe wake deadline** without async: blocked `ReadFile` has no timeout; needs overlapped io or `CancelSynchronousIo` watchdog; a bug here = hung wake thread (not hung ui). test the hostile cases.
4. **clipboard contention**: other apps hold the clipboard open → must retry briefly and fail loud-plain; DIB→rgba conversion edge cases (BI_BITFIELDS, V5 alpha, bottom-up).
5. **updater on windows**: av/defender locking the new file; running-exe rename dance; mutex handoff on restart. highest-consequence code (a bad swap bricks the app) → keep rename-back rollback + signature check first, test with deliberate failures.
6. **smartscreen/defender false positives** on an unsigned rust exe → first-run friction, not a code risk; track download-to-run drop-off; budget for a signing cert if it hurts.
7. **tray balloon notifications** may be suppressed by focus assist/do-not-disturb (same as any toast) and Win11 groups them under the exe's name; need a hidden window + message loop thread owned by us (small but not zero idle cost — keep it parked in `GetMessage`, created lazily).
8. **no windows machine in the loop for iteration** except heatpc reboot → lean on the wine gate to keep the manual cycle short.
9. **scope creep**: the owner's non-negotiables (no gpu, vanilla, minimal) rule out a windows toast crate, rfd, arboard, rodio; keep every replacement ≤ ~100 lines of windows-sys.
10. **the hidden-console choice**: with `windows_subsystem="windows"` all `eprintln!` diagnostics vanish for double-click users; error paths that matter (can't open window, already open, bad config) need a message box or notification → audit each `eprintln!` in main/app.rs when doing phase 1.
