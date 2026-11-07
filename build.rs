// the exe's icon and version info on windows. nothing at all on other targets.
// explorer and the taskbar read them from a resource section in the exe: an icon (the app's own logo, from
// assets/icon-*.png) and a version block (name, version, what it is). both are written here at build time from
// Cargo.toml's version and the logo files, and compiled with whichever resource compiler is on the machine:
// `zig rc` (the release machine has zig), else a windres (llvm-windres, mingw's). no crate, nothing committed.
// without any of them the exe still builds and runs, only with the generic icon: a warning says so
use std::path::{Path, PathBuf};
use std::process::Command;

// an .ico holding the png files as they are (windows vista and later read png-compressed icons)
fn ico(pngs: &[Vec<u8>]) -> Vec<u8> {
  let mut head = vec![0, 0, 1, 0];
  head.extend((pngs.len() as u16).to_le_bytes());
  let mut data: Vec<u8> = Vec::new();
  let first = 6 + 16 * pngs.len();
  for p in pngs {
    // the png's own size is in its IHDR: width at byte 16, height at 20 (big endian); 0 means 256
    let size = |at: usize| match u32::from_be_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]]) {
      256.. => 0,
      n => n as u8,
    };
    head.extend([size(16), size(20), 0, 0, 1, 0, 32, 0]);
    head.extend((p.len() as u32).to_le_bytes());
    head.extend(((first + data.len()) as u32).to_le_bytes());
    data.extend(p.iter());
  }
  head.extend(data);
  head
}

fn rc(version: &str, ico: &Path) -> String {
  let mut n = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).chain(std::iter::repeat(0));
  let v = format!("{},{},{},{}", n.next().unwrap(), n.next().unwrap(), n.next().unwrap(), 0);
  format!(
    r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {v}
PRODUCTVERSION {v}
FILEOS 0x40004
FILETYPE 1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "twitch, kick and youtube chat in one window"
      VALUE "ProductName", "heatsync-chat"
      VALUE "InternalName", "heatsync-chat"
      VALUE "OriginalFilename", "heatsync-chat.exe"
      VALUE "CompanyName", "heatsync"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
    ico = ico.display().to_string().replace('\\', "/")
  )
}

fn run(cmd: &str, args: &[&str]) -> bool {
  Command::new(cmd).args(args).status().is_ok_and(|s| s.success())
}

fn main() {
  println!("cargo:rerun-if-changed=build.rs");
  println!("cargo:rerun-if-changed=assets");
  if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
    return;
  }
  let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
  let pngs: Vec<Vec<u8>> = ["icon-48.png", "icon-96.png"]
    .iter()
    .map(|f| std::fs::read(Path::new("assets").join(f)).expect("the logo files are in assets/"))
    .collect();
  let (ico_path, rc_path) = (out.join("heatsync-chat.ico"), out.join("heatsync-chat.rc"));
  std::fs::write(&ico_path, ico(&pngs)).expect("write the icon");
  std::fs::write(&rc_path, rc(&std::env::var("CARGO_PKG_VERSION").unwrap_or_default(), &ico_path))
    .expect("write the resource script");
  let (rc_s, res, obj) = (rc_path.to_string_lossy(), out.join("heatsync-chat.res"), out.join("heatsync-chat.res.o"));
  let (res_s, obj_s) = (res.to_string_lossy(), obj.to_string_lossy());
  // zig's compiler makes a .res (its linker takes that); windres makes a coff object (any linker takes that)
  let linked = if run("zig", &["rc", &rc_s, &res_s]) {
    Some(res)
  } else {
    ["llvm-windres", "x86_64-w64-mingw32-windres", "windres"]
      .iter()
      .any(|w| run(w, &["-O", "coff", "-i", &rc_s, "-o", &obj_s]))
      .then_some(obj)
  };
  match linked {
    Some(file) => println!("cargo:rustc-link-arg-bins={}", file.display()),
    None => println!("cargo:warning=no resource compiler (zig or windres): the exe gets no icon or version info"),
  }
}
