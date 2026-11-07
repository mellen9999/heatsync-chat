// the font family: a name in settings, looked up off the ui thread (net/font.rs → util/font.rs), then every line
// laid out again in it. a font that can't be had says so in plain words and the current one stays
use super::App;
use crate::net::Cmd;
use crate::util::font::Found;

impl App {
  // the saved name isn't the font text is set in right now: a lookup failed (or hasn't answered yet)
  pub(super) fn font_stale(&self) -> bool {
    self.res.text.family().unwrap_or("") != self.set.font_family.as_str()
  }

  // the family in settings changed (`typed`: someone just set it, so a failure takes it back) or it is the saved one
  // at start (a failure leaves it saved: the font may be on a drive that isn't there today). empty = the system
  // monospace again, at once; a name is asked of fontconfig and answered by `font_found`
  pub(super) fn font_follow(&mut self, typed: bool) {
    let name = self.set.font_family.as_str();
    if name.is_empty() {
      self.font_ask = None;
      if self.res.text.family().is_some() {
        self.res.text.use_default();
        self.relayout_all();
      }
      return;
    }
    if self.res.text.family() == Some(name) {
      self.font_ask = None;
      return;
    }
    let name: Box<str> = name.into();
    self.font_ask = Some((name.clone(), typed));
    self.net.send(Cmd::Font(name));
  }

  // fontconfig answered for `asked`. an answer for a name that isn't the latest one asked is dropped
  pub(super) fn font_found(&mut self, asked: Box<str>, found: Result<Found, String>) {
    let Some((_, typed)) = self.font_ask.take_if(|(want, _)| *want == asked) else { return };
    let found = found.and_then(|f| {
      let mut keep = self.set.font_family;
      if !keep.set(&f.family) {
        return Err(format!("the font {asked} has a name too long to keep"));
      }
      if !self.res.text.use_family(&f.family, &f.file) {
        return Err(format!("couldn't load the font {asked}"));
      }
      Ok(keep)
    });
    match found {
      Ok(keep) => {
        // saved under the name the font files give it, so the next start asks for exactly that
        self.set.font_family = keep;
        self.set.save();
        self.relayout_all();
        self.say(format!("font: {}", keep.as_str()));
      }
      Err(why) => {
        let now = self.res.text.family().map_or("the system monospace", |f| f);
        if typed {
          // what is set is what is shown: the name that failed goes, the working font stays
          self.set.font_family.set(self.res.text.family().unwrap_or(""));
          self.set.save();
        }
        self.say(format!("{why}: still using {now}"));
      }
    }
  }
}
