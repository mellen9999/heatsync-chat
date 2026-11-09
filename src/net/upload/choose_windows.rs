// the file dialog on windows: the system's own open box (GetOpenFileNameW, comdlg32). no process, no shell.
// it blocks until the box closes, like zenity does, so the caller runs it off the ui thread. a failure to open
// at all falls back to typing a path in the input bar (`NoChooser`), the same as a unix box with no zenity
use crate::i18n::{f, t};
use std::ptr::null_mut;

use super::{Upload, parse_pick};
use crate::util::win::wide;
use windows_sys::Win32::UI::Controls::Dialogs::{
  CommDlgExtendedError, GetOpenFileNameW, OFN_DONTADDTORECENT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY,
  OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};

// what a long path can be on windows (32767 units): the buffer the box writes the choice into
const PATH_UNITS: usize = 32768;

// the box's two filters, each a label and a pattern list, the whole list ended by an extra null
fn filters() -> Vec<u16> {
  let pairs = [
    (t("net2.upload.filter_media"), "*.png;*.jpg;*.jpeg;*.gif;*.webp;*.avif;*.bmp;*.tif;*.tiff;*.mp4;*.webm;*.mov"),
    (t("net2.upload.filter_all"), "*.*"),
  ];
  let mut out = Vec::new();
  for (label, pattern) in pairs {
    out.extend(label.encode_utf16().chain(Some(0)));
    out.extend(pattern.encode_utf16().chain(Some(0)));
  }
  out.push(0);
  out
}

// blocks until the dialog closes
pub fn choose() -> Upload {
  let mut file = vec![0u16; PATH_UNITS];
  let filter = filters();
  let title = wide(t("net2.upload.dialog"));
  // SAFETY: a zeroed OPENFILENAMEW is valid (null pointers, zero flags); the buffers it points at outlive the call
  let (shown, path) = unsafe {
    let mut o: OPENFILENAMEW = std::mem::zeroed();
    o.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    o.lpstrFilter = filter.as_ptr();
    o.nFilterIndex = 1;
    o.lpstrFile = file.as_mut_ptr();
    o.nMaxFile = PATH_UNITS as u32;
    o.lpstrTitle = title.as_ptr();
    o.Flags =
      OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR | OFN_DONTADDTORECENT;
    o.hwndOwner = null_mut();
    let ok = GetOpenFileNameW(&mut o) != 0;
    (ok || CommDlgExtendedError() == 0, ok)
  };
  if !shown {
    return Upload::NoChooser;
  }
  if !path {
    return Upload::Cancelled;
  }
  let len = file.iter().position(|&u| u == 0).unwrap_or(file.len());
  // one file was asked for: whatever the box wrote is checked like a typed path (a root, no stray bytes)
  match parse_pick(&String::from_utf16_lossy(&file[..len])) {
    Some(p) => Upload::Url(p.to_string_lossy().into_owned()),
    None => Upload::Cancelled,
  }
}
