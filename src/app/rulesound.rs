// a highlight rule's own sound: a preset, or a wav file of yours, at its own volume (ext d64f2e49). the rule
// carries the choice (so it syncs with the account); the wav's path stays on this device (`Filters::rule_file`).
// a file is read once when picked, with the same bounds as the mention sound's file (`util::wav`: plain pcm,
// 2MB, 3 seconds), and again when it plays
use super::App;
use crate::model::filter;
use crate::model::rules::Pick;
use crate::util::sound::{self, Preset};

impl App {
  // /rule sound <rule> <off|preset|path>
  pub(super) fn rule_sound_set(&mut self, key: &str, what: &str) -> Result<&'static str, String> {
    let (at, id) = filter::with(|f| {
      let at = f.rules().at(key)?;
      Some((at, f.rules().id_at(at)?.to_owned()))
    })
    .ok_or("no such rule. /rule list shows the numbers")?;
    let (pick, path) = match what {
      "off" | "none" => (Pick::Off, None),
      w if Preset::parse(w).is_some() => (Pick::Preset(Preset::parse(w).ok_or("")?), None),
      path => {
        crate::util::wav::read(path).map_err(|b| format!("that file won't play: {}", b.said()))?;
        let name = path.rsplit(std::path::is_separator).next().unwrap_or(path).to_owned();
        (Pick::File(name), Some(path.to_owned()))
      }
    };
    filter::edit(|f| {
      // the file first (its id and path are checked there); the rule's change puts it back if it is refused
      let before = f.rule_file(&id).map(str::to_owned);
      if !f.set_rule_file(&id, path.as_deref()) {
        return Err("that path can't be kept".to_owned());
      }
      f.rules_mut().set_sound(key, &pick).map_err(|e| {
        f.set_rule_file(&id, before.as_deref());
        e.to_owned()
      })
    })?;
    self.rule_sound_hear(at);
    Ok(if matches!(pick, Pick::Off) { "rule sound off" } else { "rule sound set" })
  }

  // /rule volume <rule> <0-100|off>
  pub(super) fn rule_volume_set(&mut self, key: &str, pct: Option<u8>) -> Result<&'static str, String> {
    let at = filter::with(|f| f.rules().at(key)).ok_or("no such rule. /rule list shows the numbers")?;
    filter::edit(|f| f.rules_mut().set_volume(key, pct)).map_err(str::to_owned)?;
    self.rule_sound_hear(at);
    Ok(if pct.is_some() { "rule volume set" } else { "rule volume follows the mention sound" })
  }

  // the rule's sound as it plays now, heard when you set it (not while streamer mode holds sounds back)
  pub(super) fn rule_sound_hear(&self, at: usize) {
    if self.sound_muted() {
      return;
    }
    if let Some((voice, vol)) = filter::with(|f| f.rule_voice_at(at)) {
      sound::preview(voice, vol.unwrap_or_else(|| self.set.mention_volume.percent()));
    }
  }
}
