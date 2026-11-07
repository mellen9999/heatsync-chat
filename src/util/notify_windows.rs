// a notification on windows: a tray balloon (Shell_NotifyIconW; windows 10 and 11 show it as a normal toast).
// nothing exists until the first one: then a thread starts with a hidden message-only window and sleeps in
// GetMessage (idle cpu 0). the tray icon is added for a balloon and taken down ~10s after the last one, and at
// exit, so there is no icon sitting in the tray. the ui thread never waits on the shell: `show` only leaves the
// text in a slot and posts one message. the text is untrusted chat: it is cut to the balloon's buffers, shown
// as plain text, and never logged
use super::{fit, unmark};
use crate::util::win::wide;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use windows_sys::Win32::Foundation::{GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
  NIF_ICON, NIF_INFO, NIF_SHOWTIP, NIF_TIP, NIIF_USER, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
  Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
  CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HICON, HWND_MESSAGE, IDI_APPLICATION,
  KillTimer, LoadIconW, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer, TranslateMessage, WM_APP,
  WM_DESTROY, WM_TIMER, WNDCLASSW,
};

// the one tray icon's id, the message that says "a balloon is waiting", the timer that takes the icon down
const ICON_ID: u32 = 1;
const SHOW: u32 = WM_APP + 1;
const TIMER: usize = 1;
const LINGER_MS: u32 = 10_000;
// the balloon's fixed buffers, in utf-16 units, less the end mark
const TITLE_UNITS: usize = 63;
const BODY_UNITS: usize = 255;
const TIP: &str = "heatsync-chat";

// the newest balloon waiting for the window thread (a raid is already one popup: see notify::Coalesce)
static PENDING: Mutex<Option<(String, String)>> = Mutex::new(None);
// the window, once it exists (0 = not yet). set before the thread looks at PENDING, so a show is never lost
static WINDOW: AtomicIsize = AtomicIsize::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);
// the icon is in the tray. held while the shell is called, only by the window thread (and a last try at exit)
static ADDED: Mutex<bool> = Mutex::new(false);

fn take<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
  m.lock().unwrap_or_else(|e| e.into_inner())
}

// `s` into a fixed utf-16 buffer; the buffer starts zeroed, so it ends with an end mark
fn put<const N: usize>(dst: &mut [u16; N], s: &str) {
  for (d, u) in dst.iter_mut().zip(s.encode_utf16().take(N - 1)) {
    *d = u;
  }
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
  // SAFETY: plain integers, pointers and arrays: all zero is a valid, empty value
  let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
  d.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
  d.hWnd = hwnd;
  d.uID = ICON_ID;
  d
}

// the exe's own icon (resource 1, see build.rs), else the system's plain one
fn icon() -> HICON {
  // SAFETY: shared icons, never destroyed; resource 1 is an integer id passed as a "string" pointer
  unsafe {
    let me = GetModuleHandleW(null());
    let own = LoadIconW(me, std::ptr::without_provenance(1));
    if own.is_null() { LoadIconW(null_mut(), IDI_APPLICATION) } else { own }
  }
}

// add the icon (if it is not there) and show `title`/`body` on it. window thread only
fn balloon(hwnd: HWND, title: &str, body: &str) {
  let mut added = take(&ADDED);
  let mut d = data(hwnd);
  d.hIcon = icon();
  d.hBalloonIcon = d.hIcon;
  d.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
  put(&mut d.szTip, TIP);
  // SAFETY: `d` is a filled NOTIFYICONDATAW that lives through the calls
  unsafe {
    if !*added && Shell_NotifyIconW(NIM_ADD, &d) != 0 {
      *added = true;
    }
    d.uFlags |= NIF_INFO;
    put(&mut d.szInfoTitle, fit(&unmark(title), TITLE_UNITS));
    put(&mut d.szInfo, fit(&unmark(body), BODY_UNITS));
    d.dwInfoFlags = NIIF_USER;
    // the shell can lose the icon (explorer restarted): add it again once and retry
    if *added && Shell_NotifyIconW(NIM_MODIFY, &d) == 0 && Shell_NotifyIconW(NIM_ADD, &d) != 0 {
      Shell_NotifyIconW(NIM_MODIFY, &d);
    }
  }
}

fn unadd(hwnd: HWND, added: &mut bool) {
  if std::mem::take(added) {
    // SAFETY: a filled NOTIFYICONDATAW naming our own icon
    unsafe { Shell_NotifyIconW(NIM_DELETE, &data(hwnd)) };
  }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
  match msg {
    SHOW => {
      let next = take(&PENDING).take();
      if let Some((title, body)) = next {
        balloon(hwnd, &title, &body);
        // SAFETY: our own window and timer id; a second call restarts the countdown
        unsafe { SetTimer(hwnd, TIMER, LINGER_MS, None) };
      }
      0
    }
    WM_TIMER if w == TIMER => {
      // SAFETY: our own window and timer id
      unsafe { KillTimer(hwnd, TIMER) };
      unadd(hwnd, &mut take(&ADDED));
      0
    }
    WM_DESTROY => {
      unadd(hwnd, &mut take(&ADDED));
      // SAFETY: ends this thread's message loop
      unsafe { PostQuitMessage(0) };
      0
    }
    // SAFETY: the defaults, with the arguments we were given
    _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
  }
}

fn run() {
  let class = wide("heatsync-chat-tray");
  // SAFETY: a window class and a message-only window of our own, then the thread's message loop
  unsafe {
    let me = GetModuleHandleW(null());
    let mut wc: WNDCLASSW = std::mem::zeroed();
    wc.lpfnWndProc = Some(proc);
    wc.hInstance = me;
    wc.lpszClassName = class.as_ptr();
    RegisterClassW(&wc);
    let hwnd = CreateWindowExW(0, class.as_ptr(), null(), 0, 0, 0, 0, 0, HWND_MESSAGE, null_mut(), me, null());
    if hwnd.is_null() {
      eprintln!("notifications can't pop up: no window for the tray (error {})", GetLastError());
      return;
    }
    WINDOW.store(hwnd as isize, Ordering::SeqCst);
    // one that arrived while the window was being made
    if take(&PENDING).is_some() {
      PostMessageW(hwnd, SHOW, 0, 0);
    }
    let mut m: MSG = std::mem::zeroed();
    while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
      TranslateMessage(&m);
      DispatchMessageW(&m);
    }
    DestroyWindow(hwnd);
  }
}

// `_what` names the feature for the unix hint about a missing notify-send: nothing is missing here
pub fn show(_what: &str, title: &str, body: &str) {
  *take(&PENDING) = Some((title.to_owned(), body.to_owned()));
  let hwnd = WINDOW.load(Ordering::SeqCst);
  if hwnd != 0 {
    // SAFETY: posting to our own window from any thread
    unsafe { PostMessageW(hwnd as HWND, SHOW, 0, 0) };
  } else if !STARTED.swap(true, Ordering::SeqCst) {
    let _ = std::thread::Builder::new().name("tray".into()).spawn(run);
  }
}

// the icon comes out of the tray before the process ends. a shell that is busy answering the window thread is
// not waited for: the process is going either way
pub fn shutdown() {
  let hwnd = WINDOW.load(Ordering::SeqCst);
  if hwnd != 0
    && let Ok(mut added) = ADDED.try_lock()
  {
    unadd(hwnd as HWND, &mut added);
  }
}
