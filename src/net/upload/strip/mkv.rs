// webm is matroska: a tree of ebml elements. the ones that hold tags, attachments, chapters and
// the file's title / date / muxer names are overwritten with a zeroed Void element of the same
// total size, so every seek position and cluster offset in the file stays true
use super::Fail;

const SEGMENT: u32 = 0x1853_8067;
const INFO: u32 = 0x1549_A966;
const CLUSTER: u32 = 0x1F43_B675;
// tags, attachments, chapters
const DROP: [u32; 3] = [0x1254_C367, 0x1941_A469, 0x1043_A770];
// title, date, muxing app, writing app
const INFO_DROP: [u32; 4] = [0x7BA9, 0x4461, 0x4D80, 0x5741];
const MAX_ELEMENTS: usize = 5_000_000;

struct El {
  id: u32,
  start: usize,
  body: usize,
  // None: the size is unknown (a live stream), the element runs to its parent's end
  end: Option<usize>,
}

fn element(b: &[u8], at: usize, limit: usize) -> Result<El, Fail> {
  let first = *b.get(at).ok_or(Fail::Damaged)?;
  let idw = first.leading_zeros() as usize + 1;
  if idw > 4 {
    return Err(Fail::Damaged);
  }
  let mut id = 0u32;
  for i in 0..idw {
    id = (id << 8) | u32::from(*b.get(at + i).ok_or(Fail::Damaged)?);
  }
  let s0 = *b.get(at + idw).ok_or(Fail::Damaged)?;
  let sw = s0.leading_zeros() as usize + 1;
  if sw > 8 {
    return Err(Fail::Damaged);
  }
  let mut size = u64::from(s0) & ((1u64 << (8 - sw)) - 1);
  let mut ones = size == (1u64 << (8 - sw)) - 1;
  for i in 1..sw {
    let v = *b.get(at + idw + i).ok_or(Fail::Damaged)?;
    ones &= v == 0xff;
    size = (size << 8) | u64::from(v);
  }
  let body = at + idw + sw;
  let end = if ones {
    None
  } else {
    let e = body.checked_add(usize::try_from(size).map_err(|_| Fail::Damaged)?).ok_or(Fail::Damaged)?;
    if e > limit {
      return Err(Fail::Damaged);
    }
    Some(e)
  };
  Ok(El { id, start: at, body, end })
}

// a Void element (id 0xEC) over the whole element, header included; the body is zeroed
fn void(b: &mut [u8], el: &El) -> Result<(), Fail> {
  let end = el.end.ok_or(Fail::Refused)?;
  let w = el.body - el.start - 1;
  let len = (end - el.body) as u64;
  if w == 0 || w > 8 || len >= (1u64 << (7 * w)) - 1 {
    return Err(Fail::Refused);
  }
  b[el.start] = 0xec;
  let marked = len | (1u64 << (7 * w));
  for i in 0..w {
    b[el.start + 1 + i] = (marked >> (8 * (w - 1 - i))) as u8;
  }
  b[el.body..end].fill(0);
  Ok(())
}

pub fn clean(b: &mut [u8]) -> Result<(), Fail> {
  let n = b.len();
  scan(b, 0, n, &mut 0)
}

fn scan(b: &mut [u8], from: usize, to: usize, seen: &mut usize) -> Result<(), Fail> {
  let mut at = from;
  while at < to {
    *seen += 1;
    if *seen > MAX_ELEMENTS {
      return Err(Fail::Damaged);
    }
    let el = element(b, at, to)?;
    match (el.id, el.end) {
      (SEGMENT, end) => {
        scan(b, el.body, end.unwrap_or(to), seen)?;
        at = end.unwrap_or(to);
      }
      // blocks are read flat: nothing inside a cluster is on the drop list
      (CLUSTER, None) => at = el.body,
      (INFO, Some(end)) => {
        let mut c = el.body;
        while c < end {
          let ch = element(b, c, end)?;
          let ce = ch.end.ok_or(Fail::Damaged)?;
          if INFO_DROP.contains(&ch.id) {
            void(b, &ch)?;
          }
          c = ce;
        }
        at = end;
      }
      (id, _) if DROP.contains(&id) => {
        void(b, &el)?;
        at = el.end.ok_or(Fail::Refused)?;
      }
      (_, Some(end)) => at = end,
      // an unknown-size element nobody can step over
      (_, None) => return Err(Fail::Refused),
    }
  }
  Ok(())
}
