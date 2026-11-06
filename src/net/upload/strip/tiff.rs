// tiff keeps its camera, time, gps and xmp in tags. the first directory is rebuilt in place from an
// allowlist of tags a viewer needs (size, colors, compression, strips, rotation, color profile);
// every other tag's data is zeroed, and so is everything it points at (exif and gps directories,
// the later directories with their thumbnails). offsets elsewhere in the file stay true
use super::Fail;

const MAX_DEPTH: usize = 4;
const MAX_ENTRIES: usize = 4096;

const KEEP: &[u16] = &[
  254, 255, 256, 257, 258, 259, 262, 263, 264, 265, 266, 273, 274, 277, 278, 279, 280, 281, 282, 283, 284, 286, 287,
  296, 301, 317, 318, 319, 320, 322, 323, 324, 325, 332, 333, 334, 338, 339, 340, 341, 347, 512, 513, 514, 515, 517,
  518, 519, 520, 521, 529, 34675,
];
// exif, gps, interoperability, sub directories: their values are offsets of more directories
const POINTERS: [u16; 4] = [0x8769, 0x8825, 0xa005, 0x014a];

struct Tiff<'a> {
  b: &'a mut [u8],
  le: bool,
}

#[derive(Clone, Copy)]
struct Entry {
  tag: u16,
  ty: u16,
  count: u32,
  at: usize,
}

impl Tiff<'_> {
  fn u16_at(&self, o: usize) -> Result<u16, Fail> {
    let s: [u8; 2] = self.b.get(o..o.checked_add(2).ok_or(Fail::Damaged)?).ok_or(Fail::Damaged)?.try_into().unwrap();
    Ok(if self.le { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) })
  }

  fn u32_at(&self, o: usize) -> Result<u32, Fail> {
    let s: [u8; 4] = self.b.get(o..o.checked_add(4).ok_or(Fail::Damaged)?).ok_or(Fail::Damaged)?.try_into().unwrap();
    Ok(if self.le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
  }

  fn put32(&mut self, o: usize, v: u32) {
    let s = if self.le { v.to_le_bytes() } else { v.to_be_bytes() };
    self.b[o..o + 4].copy_from_slice(&s);
  }

  // the entries of the directory at `off` and where its next-directory pointer sits
  fn ifd(&self, off: usize) -> Result<(Vec<Entry>, usize), Fail> {
    let n = usize::from(self.u16_at(off)?);
    if n > MAX_ENTRIES {
      return Err(Fail::Damaged);
    }
    let mut v = Vec::with_capacity(n);
    for k in 0..n {
      let at = off + 2 + 12 * k;
      v.push(Entry { tag: self.u16_at(at)?, ty: self.u16_at(at + 2)?, count: self.u32_at(at + 4)?, at });
    }
    let next = off + 2 + 12 * n;
    self.u32_at(next)?;
    Ok((v, next))
  }

  // where an entry's values sit: inside the entry when they fit in 4 bytes, else at its offset
  fn data(&self, e: &Entry) -> Result<(usize, usize), Fail> {
    let unit = match e.ty {
      1 | 2 | 6 | 7 => 1,
      3 | 8 => 2,
      4 | 9 | 11 | 13 => 4,
      5 | 10 | 12 => 8,
      _ => return Err(Fail::Damaged),
    };
    let len = (e.count as usize).checked_mul(unit).ok_or(Fail::Damaged)?;
    let start = if len <= 4 { e.at + 8 } else { self.u32_at(e.at + 8)? as usize };
    let end = start.checked_add(len).ok_or(Fail::Damaged)?;
    if end > self.b.len() {
      return Err(Fail::Damaged);
    }
    Ok((start, end))
  }

  // numbers out of a short or long array tag
  fn nums(&self, e: &Entry) -> Result<Vec<usize>, Fail> {
    let (s, _) = self.data(e)?;
    (0..e.count.min(65536) as usize)
      .map(|i| match e.ty {
        3 => self.u16_at(s + 2 * i).map(usize::from),
        4 => self.u32_at(s + 4 * i).map(|v| v as usize),
        _ => Err(Fail::Damaged),
      })
      .collect()
  }

  fn zero(&mut self, s: usize, e: usize) {
    self.b[s..e].fill(0);
  }

  // zero a directory, the values it points at, and (for `deep`) its thumbnail or strips
  fn wipe(&mut self, off: usize, depth: usize, deep: bool) -> Result<(), Fail> {
    if depth > MAX_DEPTH {
      return Err(Fail::Damaged);
    }
    let (entries, next) = self.ifd(off)?;
    let mut sub: Vec<usize> = Vec::new();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for e in &entries {
      let (s, en) = self.data(e)?;
      if POINTERS.contains(&e.tag) {
        sub.extend(self.nums(e)?);
      }
      if s != e.at + 8 {
        spans.push((s, en));
      }
    }
    if deep {
      let find = |t: u16| entries.iter().find(|e| e.tag == t);
      for (o, l) in [(273u16, 279u16), (513, 514)] {
        let (Some(o), Some(l)) = (find(o), find(l)) else { continue };
        for (s, n) in self.nums(o)?.into_iter().zip(self.nums(l)?) {
          let end = s.checked_add(n).ok_or(Fail::Damaged)?;
          if end > self.b.len() {
            return Err(Fail::Damaged);
          }
          spans.push((s, end));
        }
      }
    }
    for s in sub {
      self.wipe(s, depth + 1, true)?;
    }
    let after = self.u32_at(next)? as usize;
    for (s, e) in spans {
      self.zero(s, e);
    }
    self.zero(off, next + 4);
    // later directories are not the picture: gone too, thumbnails and all
    if after != 0 {
      self.wipe(after, depth + 1, true)?;
    }
    Ok(())
  }
}

pub fn clean(b: &mut [u8]) -> Result<(), Fail> {
  let le = match b.get(..2) {
    Some(b"II") => true,
    Some(b"MM") => false,
    _ => return Err(Fail::Damaged),
  };
  let mut t = Tiff { b, le };
  match t.u16_at(2)? {
    42 => {}
    // bigtiff has 8-byte offsets this doesn't walk
    43 => return Err(Fail::Refused),
    _ => return Err(Fail::Damaged),
  }
  let off = t.u32_at(4)? as usize;
  let (entries, next) = t.ifd(off)?;
  let after = t.u32_at(next)? as usize;
  let mut spans: Vec<(usize, usize)> = Vec::new();
  let mut sub: Vec<usize> = Vec::new();
  let mut kept: Vec<Entry> = Vec::new();
  for e in &entries {
    if KEEP.contains(&e.tag) {
      kept.push(*e);
      continue;
    }
    let (s, en) = t.data(e)?;
    if POINTERS.contains(&e.tag) {
      sub.extend(t.nums(e)?);
    }
    if s != e.at + 8 {
      spans.push((s, en));
    }
  }
  for s in sub {
    t.wipe(s, 1, true)?;
  }
  if after != 0 {
    t.wipe(after, 1, true)?;
  }
  for (s, e) in spans {
    t.zero(s, e);
  }
  // count, the kept entries packed up, no next directory; the rest of the old block is zero
  let old_end = next + 4;
  let raw: Vec<u8> = kept.iter().flat_map(|e| t.b[e.at..e.at + 12].to_vec()).collect();
  t.zero(off, old_end);
  let n = kept.len() as u16;
  let count = if le { n.to_le_bytes() } else { n.to_be_bytes() };
  t.b[off..off + 2].copy_from_slice(&count);
  t.b[off + 2..off + 2 + raw.len()].copy_from_slice(&raw);
  t.put32(off + 2 + raw.len(), 0);
  Ok(())
}
