// what this app says when you change your own emotes: add, remove, rename, swap, block, unblock. the words and
// the answer that says each one are one file for every app (`emote-notice.vectors.json`, vendored byte for byte
// from the site by scripts/sync-emote-vectors.sh) and run below. pure: the net layer feeds it the http status
// (0 = no answer at all) and shows what it returns, so none of this needs a network
use crate::i18n::f;

// the request an answer belongs to
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
  Add,
  Remove,
  Rename,
  Swap,
  Block,
  Unblock,
}

// one line, by the contract's id
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Say {
  AddOk,
  AddLogin,
  AddDupe,
  AddFull,
  AddBusy,
  AddFailed,
  RemoveOk,
  RemoveFailed,
  ManageLogin,
  RenameOk,
  RenameFailed,
  SwapOk,
  SwapKept,
  SwapFailed,
  BlockOk,
  BlockFailed,
  UnblockOk,
  UnblockFailed,
}

impl Say {
  // the contract's id for this line
  #[cfg(test)]
  const fn id(self) -> &'static str {
    match self {
      Say::AddOk => "add.ok",
      Say::AddLogin => "add.login",
      Say::AddDupe => "add.dupe",
      Say::AddFull => "add.full",
      Say::AddBusy => "add.busy",
      Say::AddFailed => "add.failed",
      Say::RemoveOk => "remove.ok",
      Say::RemoveFailed => "remove.failed",
      Say::ManageLogin => "manage.login",
      Say::RenameOk => "rename.ok",
      Say::RenameFailed => "rename.failed",
      Say::SwapOk => "swap.ok",
      Say::SwapKept => "swap.kept",
      Say::SwapFailed => "swap.failed",
      Say::BlockOk => "block.ok",
      Say::BlockFailed => "block.failed",
      Say::UnblockOk => "unblock.ok",
      Say::UnblockFailed => "unblock.failed",
    }
  }

  // the words, in the language in use. `{name}` is the emote, `{slot}` the slot the server filed it in
  fn key(self) -> &'static str {
    match self {
      Say::AddOk => "net2.emote.added",
      Say::AddLogin => "net2.emote.add_login",
      Say::AddDupe => "net2.emote.add_dupe",
      Say::AddFull => "net2.emote.add_full",
      Say::AddBusy => "net2.emote.add_busy",
      Say::AddFailed => "net2.emote.add_failed",
      Say::RemoveOk => "net2.emote.removed",
      Say::RemoveFailed => "net2.emote.remove_failed",
      Say::ManageLogin => "net2.emote.manage_login",
      // the inventory screen's own line: the contract's words, then its undo hint
      Say::RenameOk => "settings.inventory.renamed",
      Say::RenameFailed => "net2.emote.rename_failed",
      Say::SwapOk => "net2.emote.swapped",
      Say::SwapKept => "net2.emote.kept_upload",
      Say::SwapFailed => "net2.emote.swap_failed",
      Say::BlockOk => "app.emote.blocked",
      Say::BlockFailed => "net2.emote.block_failed",
      Say::UnblockOk => "app.emote.unblocked",
      Say::UnblockFailed => "net2.emote.unblock_failed",
    }
  }

  // the line. an add the server filed without telling which slot says it plainly, never a made-up number
  pub fn line(self, name: &str, slot: Option<u64>) -> String {
    match (self, slot) {
      (Say::AddOk, None) => f("app.emote_up.added", &[("name", name)]),
      (Say::AddOk, Some(slot)) => f(self.key(), &[("name", name), ("slot", &slot.to_string())]),
      _ => f(self.key(), &[("name", name)]),
    }
  }

  pub fn is_ok(self) -> bool {
    matches!(self, Say::AddOk | Say::RemoveOk | Say::RenameOk | Say::SwapOk | Say::BlockOk | Say::UnblockOk)
  }

  #[cfg(test)]
  const ALL: [Say; 18] = [
    Say::AddOk,
    Say::AddLogin,
    Say::AddDupe,
    Say::AddFull,
    Say::AddBusy,
    Say::AddFailed,
    Say::RemoveOk,
    Say::RemoveFailed,
    Say::ManageLogin,
    Say::RenameOk,
    Say::RenameFailed,
    Say::SwapOk,
    Say::SwapKept,
    Say::SwapFailed,
    Say::BlockOk,
    Say::BlockFailed,
    Say::UnblockOk,
    Say::UnblockFailed,
  ];
}

// the line an answer says: `status` is the http status, 0 for no answer (offline, timed out, cut off)
pub fn says(op: Op, status: u16) -> Say {
  let ok = (200..300).contains(&status);
  match op {
    Op::Add => match status {
      _ if ok => Say::AddOk,
      401 => Say::AddLogin,
      403 => Say::AddFull,
      409 => Say::AddDupe,
      503 => Say::AddBusy,
      _ => Say::AddFailed,
    },
    // a slot that is already empty is a remove that worked
    Op::Remove => match status {
      _ if ok => Say::RemoveOk,
      404 => Say::RemoveOk,
      401 => Say::ManageLogin,
      _ => Say::RemoveFailed,
    },
    Op::Rename => match status {
      _ if ok => Say::RenameOk,
      401 => Say::ManageLogin,
      // the new name is taken
      409 => Say::AddDupe,
      _ => Say::RenameFailed,
    },
    Op::Swap => match status {
      _ if ok => Say::SwapOk,
      // the emote under that name is your own upload: an upload is never swapped out
      409 => Say::SwapKept,
      _ => Say::SwapFailed,
    },
    Op::Block => {
      if ok {
        Say::BlockOk
      } else {
        Say::BlockFailed
      }
    }
    // nothing blocked there is an unblock that worked
    Op::Unblock => {
      if ok || status == 404 {
        Say::UnblockOk
      } else {
        Say::UnblockFailed
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::Value;

  const FILE: &str = include_str!("emote-notice.vectors.json");

  fn file() -> serde_json::Map<String, Value> {
    match serde_json::from_str::<Value>(FILE).expect("the vector file is json") {
      Value::Object(m) => m,
      _ => panic!("the vector file is an object"),
    }
  }

  fn op(o: &Value) -> Op {
    match o.as_str() {
      Some("add") => Op::Add,
      Some("remove") => Op::Remove,
      Some("rename") => Op::Rename,
      Some("swap") => Op::Swap,
      Some("block") => Op::Block,
      Some("unblock") => Op::Unblock,
      _ => panic!("an op this runner does not know: {o}"),
    }
  }

  fn say(id: &str) -> Say {
    Say::ALL.into_iter().find(|s| s.id() == id).unwrap_or_else(|| panic!("an id this runner does not know: {id}"))
  }

  // the file is the site's, byte for byte: its hash is kept beside it and compared across the apps
  #[test]
  fn the_vendored_vectors_are_the_ones_synced() {
    let sum = ring::digest::digest(&ring::digest::SHA256, FILE.as_bytes());
    let hex: String = sum.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, include_str!("emote-notice.vectors.sha256").trim(), "run scripts/sync-emote-vectors.sh");
  }

  #[test]
  fn the_file_has_only_what_this_runner_reads() {
    let f = file();
    assert!(
      f.keys().all(|k| ["about", "sample", "words", "answers"].contains(&k.as_str())),
      "a field this runner does not know"
    );
    assert!(f["sample"].as_object().unwrap().keys().all(|k| k == "name" || k == "slot"), "a sample field");
  }

  // every line, in english, is the contract's word for word
  #[test]
  fn every_line_says_what_the_vectors_say() {
    crate::i18n::set(crate::i18n::lang::EN);
    let f = file();
    let sample = f["sample"].as_object().unwrap();
    let (name, slot) = (sample["name"].as_str().unwrap(), sample["slot"].as_u64().unwrap());
    let words = f["words"].as_object().unwrap();
    assert_eq!(words.len(), Say::ALL.len(), "an id the file has and the app lacks, or the other way round");
    for (id, want) in words {
      let want = want.as_str().unwrap().replace("{name}", name).replace("{slot}", &slot.to_string());
      let got = say(id).line(name, Some(slot));
      // the inventory screen's rename line carries its own undo hint after the contract's words
      if id == "rename.ok" {
        assert!(got.starts_with(&format!("{want}: ")), "{id}: {got:?} should open with {want:?}");
      } else {
        assert_eq!(got, want, "{id}");
      }
    }
  }

  #[test]
  fn every_answer_says_the_line_the_vectors_say() {
    let f = file();
    let rows = f["answers"].as_array().expect("answers");
    assert!(rows.len() >= 30, "only {} answers", rows.len());
    for r in rows {
      let o = r.as_object().expect("an answer is an object");
      assert!(o.keys().all(|k| ["op", "status", "says"].contains(&k.as_str())), "a field this runner does not know");
      let status = u16::try_from(o["status"].as_u64().expect("status")).expect("a status fits");
      let want = say(o["says"].as_str().expect("says"));
      assert_eq!(says(op(&o["op"]), status), want, "{} {status}", o["op"]);
    }
  }

  // an answer the table does not list is the op's failed line, never an ok one
  #[test]
  fn an_unlisted_answer_is_a_failure() {
    for (o, bad) in [(Op::Add, Say::AddFailed), (Op::Remove, Say::RemoveFailed), (Op::Rename, Say::RenameFailed)] {
      for status in [0, 302, 418, 502] {
        assert_eq!(says(o, status), bad, "{o:?} {status}");
      }
    }
    for status in [0, 302, 418, 502] {
      assert_eq!(says(Op::Swap, status), Say::SwapFailed);
      assert_eq!(says(Op::Block, status), Say::BlockFailed);
      assert_eq!(says(Op::Unblock, status), Say::UnblockFailed);
    }
  }

  #[test]
  fn an_add_without_a_slot_says_added_and_no_number() {
    crate::i18n::set(crate::i18n::lang::EN);
    assert_eq!(Say::AddOk.line("MeN", None), "added MeN");
    assert_eq!(Say::AddOk.line("MeN", Some(7)), "added MeN to slot 7");
  }

  // a translation keeps the holes its contract line has: the name where there is one, the slot only in an add
  #[test]
  fn every_language_keeps_the_holes_of_every_line() {
    let f = file();
    let words = f["words"].as_object().unwrap();
    for (i, code) in crate::i18n::lang::CODES.iter().enumerate() {
      for s in Say::ALL {
        let text = crate::i18n::lookup_in(i, s.key()).unwrap_or_else(|| panic!("{code} lacks {}", s.key()));
        let want = words[s.id()].as_str().unwrap();
        for hole in ["{name}", "{slot}"] {
          // the rename line is the inventory screen's, which says its hint after the words
          assert_eq!(text.contains(hole), want.contains(hole), "{code}: {} and {hole}", s.key());
        }
      }
    }
  }
}
