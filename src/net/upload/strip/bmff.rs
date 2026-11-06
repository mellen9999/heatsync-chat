// mp4, mov and avif share one box layout. a phone video keeps its gps in udta (©xyz), in meta
// (quicktime location keys, xmp), in a uuid box (xmp), or in a timed-metadata track whose samples
// sit in mdat. everything of that kind is blanked IN PLACE: the box keeps its size and becomes
// 'free' with a zeroed body, so every offset in the file (stco, iloc, moof) stays true and the
// picture and sound are never touched. avif keeps its meta box (the picture's index lives there)
// and only its exif / xmp items are zeroed and renamed
use super::Fail;

const MAX_DEPTH: usize = 8;
const MAX_BOXES: usize = 200_000;

#[derive(Clone, Copy)]
struct Bx {
  start: usize,
  hdr: usize,
  end: usize,
  kind: [u8; 4],
}

impl Bx {
  fn body(&self) -> usize {
    self.start + self.hdr
  }
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
  Some(u32::from_be_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn be_n(b: &[u8], at: usize, n: usize) -> Option<u64> {
  if n > 8 {
    return None;
  }
  let s = b.get(at..at.checked_add(n)?)?;
  Some(s.iter().fold(0u64, |a, x| (a << 8) | u64::from(*x)))
}

// the box at `at`, which must fit inside `limit`
fn next_box(b: &[u8], at: usize, limit: usize) -> Result<Bx, Fail> {
  let size = be32(b, at).ok_or(Fail::Damaged)?;
  let kind: [u8; 4] = b.get(at + 4..at + 8).ok_or(Fail::Damaged)?.try_into().map_err(|_| Fail::Damaged)?;
  let (hdr, len) = match size {
    0 => (8, limit - at),
    1 => (16, usize::try_from(be_n(b, at + 8, 8).ok_or(Fail::Damaged)?).map_err(|_| Fail::Damaged)?),
    n => (8, n as usize),
  };
  let end = at.checked_add(len).ok_or(Fail::Damaged)?;
  if len < hdr || end > limit {
    return Err(Fail::Damaged);
  }
  Ok(Bx { start: at, hdr, end, kind })
}

// the boxes between from..to; a tail too short to be a box must be zero padding
fn children(b: &[u8], from: usize, to: usize, seen: &mut usize) -> Result<Vec<Bx>, Fail> {
  if from > to || to > b.len() {
    return Err(Fail::Damaged);
  }
  let mut v = Vec::new();
  let mut at = from;
  while to - at >= 8 {
    *seen += 1;
    if *seen > MAX_BOXES {
      return Err(Fail::Damaged);
    }
    let bx = next_box(b, at, to)?;
    at = bx.end;
    v.push(bx);
  }
  if b[at..to].iter().any(|x| *x != 0) {
    return Err(Fail::Damaged);
  }
  Ok(v)
}

fn blank(b: &mut [u8], bx: &Bx) {
  b[bx.start + 4..bx.start + 8].copy_from_slice(b"free");
  b[bx.body()..bx.end].fill(0);
}

// the first child of `bx` of this kind (bx's body is all boxes)
fn find(b: &[u8], bx: &Bx, kind: &[u8; 4]) -> Result<Option<Bx>, Fail> {
  Ok(children(b, bx.body(), bx.end, &mut 0)?.into_iter().find(|c| &c.kind == kind))
}

pub fn clean(b: &mut [u8], avif: bool) -> Result<(), Fail> {
  let mut st = State { seen: 0, meta_track: false, fragmented: false, avif };
  st.walk(b, 0, b.len(), 0)?;
  // the samples of a blanked track can't be found inside a fragment
  if st.meta_track && st.fragmented { Err(Fail::Refused) } else { Ok(()) }
}

struct State {
  seen: usize,
  meta_track: bool,
  fragmented: bool,
  avif: bool,
}

impl State {
  fn walk(&mut self, b: &mut [u8], from: usize, to: usize, depth: usize) -> Result<(), Fail> {
    if depth > MAX_DEPTH {
      return Err(Fail::Damaged);
    }
    for bx in children(b, from, to, &mut self.seen)? {
      match &bx.kind {
        b"udta" | b"uuid" | b"XMP_" | b"Exif" => blank(b, &bx),
        b"meta" if self.avif && depth == 0 => self.items(b, &bx)?,
        b"meta" => blank(b, &bx),
        b"free" | b"skip" => b[bx.body()..bx.end].fill(0),
        k if k[0] == 0xa9 => blank(b, &bx),
        b"trak" => {
          if is_meta_track(b, &bx)? {
            blank_samples(b, &bx)?;
            self.meta_track = true;
            blank(b, &bx);
          } else {
            self.walk(b, bx.body(), bx.end, depth + 1)?;
          }
        }
        b"moof" => {
          self.fragmented = true;
          self.walk(b, bx.body(), bx.end, depth + 1)?;
        }
        b"moov" | b"mdia" | b"minf" | b"traf" | b"edts" | b"mvex" => self.walk(b, bx.body(), bx.end, depth + 1)?,
        _ => {}
      }
    }
    Ok(())
  }

  // avif: the exif and xmp items, found through iinf and iloc, are zeroed and stop being exif
  fn items(&mut self, b: &mut [u8], meta: &Bx) -> Result<(), Fail> {
    // meta is a full box: version and flags first
    let kids = children(b, meta.body() + 4, meta.end, &mut self.seen)?;
    let get = |k: &[u8; 4]| kids.iter().find(|c| &c.kind == k).copied();
    let (Some(iinf), Some(iloc)) = (get(b"iinf"), get(b"iloc")) else { return Ok(()) };
    let idat = get(b"idat");
    // (item id, where its type sits)
    let mut hidden: Vec<(u32, usize)> = Vec::new();
    let v = *b.get(iinf.body()).ok_or(Fail::Damaged)?;
    let first = iinf.body() + 4 + if v == 0 { 2 } else { 4 };
    for infe in children(b, first, iinf.end, &mut self.seen)? {
      let at = infe.body();
      let ver = *b.get(at).ok_or(Fail::Damaged)?;
      // versions below 2 have no item type
      if &infe.kind != b"infe" || ver < 2 {
        continue;
      }
      let idw = if ver == 2 { 2 } else { 4 };
      let id = be_n(b, at + 4, idw).ok_or(Fail::Damaged)? as u32;
      let ty = at + 4 + idw + 2;
      if let b"Exif" | b"mime" | b"uri " = b.get(ty..ty + 4).ok_or(Fail::Damaged)? {
        hidden.push((id, ty));
      }
    }
    let mut zero: Vec<(usize, usize)> = Vec::new();
    let ver = *b.get(iloc.body()).ok_or(Fail::Damaged)?;
    let mut p = iloc.body() + 4;
    let sizes = be_n(b, p, 2).ok_or(Fail::Damaged)?;
    let (off_w, len_w, base_w) = ((sizes >> 12) as usize & 15, (sizes >> 8) as usize & 15, (sizes >> 4) as usize & 15);
    let idx_w = if ver == 0 { 0 } else { sizes as usize & 15 };
    p += 2;
    let (idw, count) = if ver < 2 { (2, be_n(b, p, 2)) } else { (4, be_n(b, p, 4)) };
    p += idw;
    for _ in 0..count.ok_or(Fail::Damaged)?.min(100_000) {
      let id = be_n(b, p, idw).ok_or(Fail::Damaged)? as u32;
      p += idw;
      let mut method = 0;
      if ver > 0 {
        method = be_n(b, p, 2).ok_or(Fail::Damaged)? & 15;
        p += 2;
      }
      p += 2;
      let base = be_n(b, p, base_w).ok_or(Fail::Damaged)?;
      p += base_w;
      let extents = be_n(b, p, 2).ok_or(Fail::Damaged)?;
      p += 2;
      for _ in 0..extents {
        p += idx_w;
        let off = be_n(b, p, off_w).ok_or(Fail::Damaged)?;
        p += off_w;
        let len = be_n(b, p, len_w).ok_or(Fail::Damaged)?;
        p += len_w;
        if !hidden.iter().any(|h| h.0 == id) {
          continue;
        }
        let start = match method {
          0 => base.checked_add(off),
          1 => idat.and_then(|d| base.checked_add(off)?.checked_add(d.body() as u64)),
          _ => return Err(Fail::Refused),
        };
        let start = usize::try_from(start.ok_or(Fail::Damaged)?).map_err(|_| Fail::Damaged)?;
        let end = start.checked_add(usize::try_from(len).map_err(|_| Fail::Damaged)?).ok_or(Fail::Damaged)?;
        if end > b.len() {
          return Err(Fail::Damaged);
        }
        zero.push((start, end));
      }
    }
    for (s, e) in zero {
      b[s..e].fill(0);
    }
    for (_, ty) in hidden {
      b[ty..ty + 4].copy_from_slice(b"free");
    }
    Ok(())
  }
}

// a track whose handler is 'meta' carries timed metadata (gps fixes, motion), not picture or sound
fn is_meta_track(b: &[u8], trak: &Bx) -> Result<bool, Fail> {
  let Some(mdia) = find(b, trak, b"mdia")? else { return Ok(false) };
  let Some(hdlr) = find(b, &mdia, b"hdlr")? else { return Ok(false) };
  // version/flags, predefined, then the handler type
  Ok(b.get(hdlr.body() + 8..hdlr.body() + 12) == Some(b"meta"))
}

// zero the samples a track points at (stsc says how many per chunk, stsz how big, stco/co64 where)
fn blank_samples(b: &mut [u8], trak: &Bx) -> Result<(), Fail> {
  let mut cur = *trak;
  for k in [b"mdia", b"minf", b"stbl"] {
    match find(b, &cur, k)? {
      Some(c) => cur = c,
      None => return Ok(()),
    }
  }
  let stbl = cur;
  let (Some(stsz), Some(stsc)) = (find(b, &stbl, b"stsz")?, find(b, &stbl, b"stsc")?) else { return Ok(()) };
  let (co, wide) = match (find(b, &stbl, b"stco")?, find(b, &stbl, b"co64")?) {
    (Some(c), _) => (c, false),
    (None, Some(c)) => (c, true),
    _ => return Ok(()),
  };
  let all = be32(b, stsz.body() + 4).ok_or(Fail::Damaged)? as usize;
  let n_samples = be32(b, stsz.body() + 8).ok_or(Fail::Damaged)? as usize;
  let size_of = |i: usize| -> Result<usize, Fail> {
    if all != 0 {
      return Ok(all);
    }
    be32(b, stsz.body() + 12 + 4 * i).map(|v| v as usize).ok_or(Fail::Damaged)
  };
  let runs = be32(b, stsc.body() + 4).ok_or(Fail::Damaged)? as usize;
  let chunks = be32(b, co.body() + 4).ok_or(Fail::Damaged)? as usize;
  if chunks > b.len() || runs > b.len() || n_samples > b.len() * 8 {
    return Err(Fail::Damaged);
  }
  let (mut sample, mut run) = (0usize, 0usize);
  let mut spans: Vec<(usize, usize)> = Vec::new();
  for chunk in 1..=chunks {
    // the last run that started at or before this chunk
    while run + 1 < runs && be32(b, stsc.body() + 8 + 12 * (run + 1)).ok_or(Fail::Damaged)? as usize <= chunk {
      run += 1;
    }
    let per = if runs == 0 { 0 } else { be32(b, stsc.body() + 12 + 12 * run).ok_or(Fail::Damaged)? as usize };
    let off =
      if wide { be_n(b, co.body() + 8 + 8 * (chunk - 1), 8) } else { be_n(b, co.body() + 8 + 4 * (chunk - 1), 4) }
        .ok_or(Fail::Damaged)?;
    let mut len = 0usize;
    for _ in 0..per {
      if sample >= n_samples {
        break;
      }
      len = len.checked_add(size_of(sample)?).ok_or(Fail::Damaged)?;
      sample += 1;
    }
    let s = usize::try_from(off).map_err(|_| Fail::Damaged)?;
    let e = s.checked_add(len).ok_or(Fail::Damaged)?;
    if e > b.len() {
      return Err(Fail::Damaged);
    }
    spans.push((s, e));
  }
  for (s, e) in spans {
    b[s..e].fill(0);
  }
  Ok(())
}
