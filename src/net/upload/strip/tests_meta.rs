// synthetic mp4 / mov / avif / tiff / webm files with a location planted in every place a camera
// puts one: after clean the location bytes are gone and the structure still parses
use super::*;

const LOC: &[u8] = b"+37.7749-122.4194";

fn find(hay: &[u8], needle: &[u8]) -> bool {
  hay.windows(needle.len()).any(|w| w == needle)
}

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
  let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
  v.extend_from_slice(kind);
  v.extend_from_slice(body);
  v
}

fn full(kind: &[u8; 4], ver: u8, body: &[u8]) -> Vec<u8> {
  let mut b = vec![ver, 0, 0, 0];
  b.extend_from_slice(body);
  bx(kind, &b)
}

fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
  parts.concat()
}

// every box kind in the tree, checking each size lines up exactly with its parent
fn kinds(b: &[u8], out: &mut Vec<String>) {
  let mut at = 0;
  while at + 8 <= b.len() {
    let n = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
    assert!(n >= 8 && at + n <= b.len(), "bad box size at {at}");
    let k = String::from_utf8_lossy(&b[at + 4..at + 8]).into_owned();
    out.push(k.clone());
    if ["moov", "trak", "mdia", "minf", "stbl"].contains(&k.as_str()) {
      kinds(&b[at + 8..at + n], out);
    }
    at += n;
  }
  assert_eq!(at, b.len());
}

fn tree(b: &[u8]) -> Vec<String> {
  let mut v = Vec::new();
  kinds(b, &mut v);
  v
}

// a video track, and (when `meta`) a timed-metadata track whose two 8-byte samples sit in mdat at `at`
fn trak(handler: &[u8; 4], at: u32) -> Vec<u8> {
  let hdlr = full(b"hdlr", 0, &cat(&[vec![0; 4], handler.to_vec(), vec![0; 13]]));
  let stsz = full(
    b"stsz",
    0,
    &cat(&[
      0u32.to_be_bytes().to_vec(),
      2u32.to_be_bytes().to_vec(),
      5u32.to_be_bytes().to_vec(),
      11u32.to_be_bytes().to_vec(),
    ]),
  );
  let stsc = full(
    b"stsc",
    0,
    &cat(&[
      1u32.to_be_bytes().to_vec(),
      1u32.to_be_bytes().to_vec(),
      2u32.to_be_bytes().to_vec(),
      1u32.to_be_bytes().to_vec(),
    ]),
  );
  let stco = full(b"stco", 0, &cat(&[1u32.to_be_bytes().to_vec(), at.to_be_bytes().to_vec()]));
  let stbl = bx(b"stbl", &cat(&[stsz, stsc, stco]));
  let mdia = bx(b"mdia", &cat(&[hdlr, bx(b"minf", &stbl)]));
  bx(b"trak", &cat(&[full(b"tkhd", 0, &[0; 80]), mdia]))
}

// ftyp, moov(mvhd, video track, [gps track], udta with ©xyz, meta with a location key), uuid xmp, mdat
fn mov(gps_track: bool) -> Vec<u8> {
  let build = |at: u32| {
    let mut moov = vec![full(b"mvhd", 0, &[0; 96]), trak(b"vide", at)];
    if gps_track {
      moov.push(trak(b"meta", at + 16));
    }
    let xyz = bx(b"\xa9xyz", &cat(&[vec![0, 17, 0x15, 0xc7], LOC.to_vec()]));
    moov.push(bx(b"udta", &xyz));
    moov.push(full(b"meta", 0, &bx(b"keys", b"com.apple.quicktime.location.ISO6709 +37.7749-122.4194")));
    cat(&[
      bx(b"ftyp", b"qt  \0\0\0\0qt  "),
      bx(b"moov", &cat(&moov)),
      bx(b"uuid", b"\xbe\x7a\xcf\xcb<x:xmpmeta>GPSLatitude</x:xmpmeta>"),
    ])
  };
  let at = build(0).len() as u32 + 8;
  let mut f = build(at);
  // video samples (5 + 11 bytes), then the gps track's (5 + 11) holding the fix
  let mut mdat = b"VIDEOFRAMES-----".to_vec();
  mdat.extend_from_slice(if gps_track { b"+37.77-122.41---" } else { b"0000000000000000" });
  f.extend_from_slice(&bx(b"mdat", &mdat));
  f
}

#[test]
fn mov_loses_location_boxes_and_keeps_every_offset() {
  let f = mov(false);
  assert!(find(&f, LOC) && find(&f, b"GPSLatitude") && find(&f, b"location.ISO6709"));
  let out = clean("video/quicktime", f.clone()).unwrap();
  assert!(!find(&out, LOC) && !find(&out, b"GPSLatitude") && !find(&out, b"location.ISO6709"));
  assert_eq!(out.len(), f.len(), "nothing moved");
  let t = tree(&out);
  for k in ["ftyp", "moov", "trak", "mdia", "stbl", "stco", "mdat"] {
    assert!(t.iter().any(|x| x == k), "{k} lost: {t:?}");
  }
  assert!(!t.iter().any(|x| x == "udta" || x == "meta" || x == "uuid" || x == "\u{fffd}xyz"));
  // the mdat is byte for byte what it was
  assert_eq!(out[out.len() - 32..], f[f.len() - 32..]);
  // clean once, clean again
  assert_eq!(clean("video/quicktime", out.clone()).unwrap(), out);
  // mp4 takes the same road
  assert!(!find(&clean("video/mp4", f).unwrap(), LOC));
}

#[test]
fn a_timed_metadata_track_loses_its_samples_and_its_place() {
  let f = mov(true);
  assert!(find(&f, b"+37.77-122.41"));
  let out = clean("video/mp4", f.clone()).unwrap();
  assert!(!find(&out, b"+37.77-122.41"), "the fixes in mdat are zeroed");
  assert!(find(&out, b"VIDEOFRAMES"), "the video samples are not");
  assert_eq!(out.len(), f.len());
  assert_eq!(tree(&out).iter().filter(|k| *k == "trak").count(), 1);
  assert_eq!(clean("video/mp4", out.clone()).unwrap(), out);
}

#[test]
fn a_fragmented_file_with_a_metadata_track_is_refused() {
  let mut f = mov(true);
  f.extend_from_slice(&bx(b"moof", &full(b"mfhd", 0, &[0; 4])));
  let e = clean("video/mp4", f).unwrap_err();
  assert!(e.contains("hidden location info"), "{e}");
}

#[test]
fn hostile_boxes_are_damaged_never_a_panic() {
  let f = mov(true);
  for n in 0..f.len() {
    let _ = clean("video/mp4", f[..n].to_vec());
  }
  // a size bigger than the file, a size smaller than a header, boxes nested past the bound
  for bad in [
    cat(&[bx(b"ftyp", b"isom"), 9999u32.to_be_bytes().to_vec(), b"moov".to_vec()]),
    cat(&[bx(b"ftyp", b"isom"), 4u32.to_be_bytes().to_vec(), b"moov".to_vec(), vec![0; 8]]),
    cat(&[bx(b"ftyp", b"isom"), b"trailing garbage here".to_vec()]),
  ] {
    assert!(clean("video/mp4", bad).unwrap_err().contains("damaged"));
  }
  let mut deep = bx(b"free", b"x");
  for _ in 0..20 {
    deep = bx(b"moov", &deep);
  }
  assert!(clean("video/mp4", cat(&[bx(b"ftyp", b"isom"), deep])).is_err());
}

// ---- avif

fn avif() -> Vec<u8> {
  let exif = b"Exif\0\0\0\0MM*GPSLatitude=37.77".to_vec();
  let build = |img_at: u32, exif_at: u32| {
    let infe =
      |id: u16, ty: &[u8; 4]| full(b"infe", 2, &cat(&[id.to_be_bytes().to_vec(), vec![0, 0], ty.to_vec(), vec![0]]));
    let iinf = full(b"iinf", 0, &cat(&[2u16.to_be_bytes().to_vec(), infe(1, b"av01"), infe(2, b"Exif")]));
    let item = |id: u16, at: u32, len: u32| {
      cat(&[id.to_be_bytes().to_vec(), vec![0, 0, 0, 1], at.to_be_bytes().to_vec(), len.to_be_bytes().to_vec()])
    };
    let iloc = full(
      b"iloc",
      0,
      &cat(&[vec![0x44, 0], 2u16.to_be_bytes().to_vec(), item(1, img_at, 9), item(2, exif_at, exif.len() as u32)]),
    );
    let meta = full(b"meta", 0, &cat(&[full(b"hdlr", 0, &[0; 20]), full(b"pitm", 0, &[0, 1]), iinf, iloc]));
    cat(&[bx(b"ftyp", b"avif\0\0\0\0avifmif1"), meta])
  };
  let img_at = build(0, 0).len() as u32 + 8;
  let mut f = build(img_at, img_at + 9);
  f.extend_from_slice(&bx(b"mdat", &cat(&[b"AV1IMAGE!".to_vec(), exif])));
  f
}

#[test]
fn avif_loses_its_exif_item_but_keeps_the_picture_and_the_index() {
  let f = avif();
  assert!(find(&f, b"GPSLatitude"));
  let out = clean("image/avif", f.clone()).unwrap();
  assert!(!find(&out, b"GPSLatitude") && !find(&out, b"MM*"));
  assert!(find(&out, b"AV1IMAGE!"));
  assert_eq!(out.len(), f.len());
  assert!(find(&out, b"av01") && !find(&out, b"Exif"));
  let t = tree(&out);
  assert_eq!(t, ["ftyp", "meta", "mdat"]);
  assert_eq!(clean("image/avif", out.clone()).unwrap(), out);
  for n in 0..f.len() {
    let _ = clean("image/avif", f[..n].to_vec());
  }
}

// ---- tiff

fn tiff(le: bool) -> Vec<u8> {
  let w16 = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() }.to_vec();
  let w32 = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() }.to_vec();
  let entry = |tag: u16, ty: u16, count: u32, val: Vec<u8>| cat(&[w16(tag), w16(ty), w32(count), val]);
  let mut f = if le { b"II".to_vec() } else { b"MM".to_vec() };
  f.extend_from_slice(&w16(42));
  f.extend_from_slice(&w32(8));
  // ifd0 at 8 (7 entries, ends at 98); make at 98 (11), date at 109 (20), gps ifd at 129, strip at 171
  f.extend_from_slice(&w16(7));
  f.extend_from_slice(&entry(256, 3, 1, cat(&[w16(4), vec![0, 0]])));
  f.extend_from_slice(&entry(257, 3, 1, cat(&[w16(1), vec![0, 0]])));
  f.extend_from_slice(&entry(271, 2, 11, w32(98)));
  f.extend_from_slice(&entry(273, 4, 1, w32(171)));
  f.extend_from_slice(&entry(279, 4, 1, w32(16)));
  f.extend_from_slice(&entry(306, 2, 20, w32(109)));
  f.extend_from_slice(&entry(34853, 4, 1, w32(129)));
  f.extend_from_slice(&w32(0));
  f.extend_from_slice(b"SECRETMAKE\0");
  f.extend_from_slice(b"2024:05:05 12:00:00\0");
  // gps directory: one rational lat (3 values, 24 bytes) after it
  f.extend_from_slice(&w16(1));
  f.extend_from_slice(&entry(2, 5, 3, w32(147)));
  f.extend_from_slice(&w32(0));
  f.extend_from_slice(b"GPSLATGPSLATGPSLATGPSLAT");
  assert_eq!(f.len(), 171);
  f.extend_from_slice(b"PIXELPIXELPIXEL!");
  f
}

fn tags(f: &[u8], le: bool) -> Vec<u16> {
  let r = |o: usize| if le { u16::from_le_bytes([f[o], f[o + 1]]) } else { u16::from_be_bytes([f[o], f[o + 1]]) };
  let off =
    if le { u32::from_le_bytes(f[4..8].try_into().unwrap()) } else { u32::from_be_bytes(f[4..8].try_into().unwrap()) }
      as usize;
  (0..r(off) as usize).map(|i| r(off + 2 + 12 * i)).collect()
}

#[test]
fn tiff_loses_camera_time_and_gps_but_keeps_the_picture() {
  for le in [true, false] {
    let f = tiff(le);
    let out = clean("image/tiff", f.clone()).unwrap();
    assert!(!find(&out, b"SECRETMAKE") && !find(&out, b"GPSLAT") && !find(&out, b"2024:05:05"));
    assert_eq!(tags(&out, le), [256, 257, 273, 279]);
    assert_eq!(out.len(), f.len());
    assert!(find(&out, b"PIXELPIXELPIXEL!"));
    assert_eq!(clean("image/tiff", out.clone()).unwrap(), out);
    for n in 0..f.len() {
      let _ = clean("image/tiff", f[..n].to_vec());
    }
  }
  // bigtiff isn't walked: refused, not sent
  let e = clean("image/tiff", b"II\x2b\0\x08\0\0\0\0\0\0\0\0\0\0\0".to_vec()).unwrap_err();
  assert!(e.contains("hidden location info"), "{e}");
}

// ---- webm

fn el(id: &[u8], body: &[u8]) -> Vec<u8> {
  let n = body.len();
  let size = if n < 127 { vec![0x80 | n as u8] } else { vec![0x40 | (n >> 8) as u8, n as u8] };
  cat(&[id.to_vec(), size, body.to_vec()])
}

fn webm(unknown: bool) -> Vec<u8> {
  let info = el(
    &[0x15, 0x49, 0xa9, 0x66],
    &cat(&[
      el(&[0x2a, 0xd7, 0xb1], &[15, 66, 64]),
      el(&[0x7b, 0xa9], b"SECRETTITLE"),
      el(&[0x44, 0x61], b"SECRETDATE"),
      el(&[0x57, 0x41], b"SECRETAPP"),
    ]),
  );
  let tags = el(&[0x12, 0x54, 0xc3, 0x67], &el(&[0x73, 0x73], b"GPS SECRETTAG"));
  let cluster = el(&[0x1f, 0x43, 0xb6, 0x75], &cat(&[el(&[0xe7], &[0]), el(&[0xa3], b"FRAME")]));
  let mut seg = cat(&[info, cluster, tags]);
  let head = el(&[0x1a, 0x45, 0xdf, 0xa3], &el(&[0x42, 0x82], b"webm"));
  let mut f = head;
  if unknown {
    f.extend_from_slice(&[0x18, 0x53, 0x80, 0x67, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    f.append(&mut seg);
  } else {
    f.extend_from_slice(&el(&[0x18, 0x53, 0x80, 0x67], &seg));
  }
  f
}

#[test]
fn webm_loses_tags_title_and_muxer_but_keeps_the_frames() {
  for unknown in [false, true] {
    let f = webm(unknown);
    assert!(find(&f, b"SECRETTAG") && find(&f, b"SECRETTITLE"));
    let out = clean("video/webm", f.clone()).unwrap();
    assert!(!find(&out, b"SECRET") && !find(&out, b"GPS"), "unknown={unknown}");
    assert!(find(&out, b"FRAME") && find(&out, b"webm"));
    assert_eq!(out.len(), f.len());
    assert_eq!(clean("video/webm", out.clone()).unwrap(), out);
    for n in 0..f.len() {
      let _ = clean("video/webm", f[..n].to_vec());
    }
  }
}
