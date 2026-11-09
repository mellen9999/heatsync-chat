// opening things on windows: programs on PATH are found by name + PATHEXT, links go to ShellExecuteW
// (no shell: the link is one string handed to the default browser's handler)
use crate::util::win::wide;
use std::ffi::OsString;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// the extensions a bare name may take, from PATHEXT (a plain `.exe` when it is unset)
fn exts(pathext: Option<OsString>) -> Vec<String> {
  let raw = pathext.and_then(|p| p.into_string().ok()).unwrap_or_default();
  let mut v: Vec<String> = raw
    .split(';')
    .filter(|e| e.starts_with('.') && e.len() > 1 && !e.contains(['\\', '/']))
    .map(str::to_ascii_lowercase)
    .collect();
  if v.is_empty() {
    v.push(".exe".into());
  }
  v
}

// the file a name means in one folder: the name as typed when it already has an extension we run, else with each extension
fn candidates(dir: &std::path::Path, bin: &str, exts: &[String]) -> Vec<PathBuf> {
  let lower = bin.to_ascii_lowercase();
  if exts.iter().any(|e| lower.ends_with(e.as_str())) {
    return vec![dir.join(bin)];
  }
  exts.iter().map(|e| dir.join(format!("{bin}{e}"))).collect()
}

// the program of that name on PATH. only PATH is searched: not the current folder, not this app's own
pub fn which(bin: &str) -> Option<PathBuf> {
  if bin.is_empty() || bin.contains(['\\', '/', ':']) {
    return None;
  }
  let path = std::env::var_os("PATH")?;
  let exts = exts(std::env::var_os("PATHEXT"));
  std::env::split_paths(&path)
    .filter(|d| d.is_absolute())
    .flat_map(|d| candidates(&d, bin, &exts))
    .find(|p| p.is_file())
}

// a program started here (mpv.exe is a console program) must not flash a console window
pub fn quiet(c: &mut Command) {
  c.creation_flags(CREATE_NO_WINDOW);
}

// the default browser (the link is already checked)
pub fn browse(url: &str) -> Result<(), String> {
  let (verb, target) = (wide("open"), wide(url));
  // SAFETY: null-terminated strings that outlive the call; no window, no folder, no arguments
  let r = unsafe {
    ShellExecuteW(
      std::ptr::null_mut(),
      verb.as_ptr(),
      target.as_ptr(),
      std::ptr::null(),
      std::ptr::null(),
      SW_SHOWNORMAL,
    )
  };
  // ShellExecute answers a number above 32 on success
  if r as usize > 32 {
    Ok(())
  } else {
    eprintln!("open link: shell error {}", r as usize);
    Err(crate::i18n::t("misc.open.no_browser").into())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::Path;

  #[test]
  fn extensions_come_from_pathext_or_default_to_exe() {
    assert_eq!(exts(Some(".COM;.EXE;.BAT".into())), [".com", ".exe", ".bat"]);
    assert_eq!(exts(None), [".exe"]);
    assert_eq!(exts(Some(";;x;..\\evil".into())), [".exe"], "junk is dropped");
  }

  #[test]
  fn a_bare_name_tries_each_extension_and_a_full_one_only_itself() {
    let e = exts(Some(".exe;.cmd".into()));
    let d = Path::new("C:\\bin");
    assert_eq!(candidates(d, "mpv", &e), [d.join("mpv.exe"), d.join("mpv.cmd")]);
    assert_eq!(candidates(d, "mpv.EXE", &e), [d.join("mpv.EXE")]);
  }

  #[test]
  fn a_path_is_not_a_program_name() {
    for bad in ["", "..\\x", "C:\\x\\mpv", "a/b", "c:mpv"] {
      assert_eq!(which(bad), None, "{bad}");
    }
  }
}
