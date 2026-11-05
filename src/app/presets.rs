// your own settings presets (ext customPresets): `/preset save <name>` keeps every switch and choice that
// differs from the defaults, `/preset <name>` applies one (a built-in or yours), `/preset del <name>` drops
// yours, and the settings screen lists them under the built-ins (enter applies, x deletes)
use super::App;
use super::screen::Target;
use crate::model::presets::{self, Cmd};
use crate::model::schema::PRESETS;

impl App {
  pub(super) fn preset_cmd(&mut self, cmd: Cmd) {
    match cmd {
      Cmd::Usage(u) => self.say(u),
      Cmd::List => {
        let own = presets::with(|c| c.all().iter().map(|p| p.name.clone()).collect::<Vec<_>>());
        let built: Vec<&str> = PRESETS.iter().map(|p| p.name).collect();
        let mut s = format!("presets: {}", built.join(", "));
        if !own.is_empty() {
          s.push_str(&format!(" · yours: {}", own.join(", ")));
        }
        self.say(s);
      }
      Cmd::Save(name) => {
        let diff = presets::snapshot(&self.set);
        let r = presets::edit(|c| c.add(&name, diff).map(|()| c.save()));
        self.say(match r {
          Ok(()) => format!("saved preset: {}", presets::clean(&name)),
          Err(e) => e.to_owned(),
        });
        self.rows_changed();
      }
      Cmd::Delete(name) => {
        let gone = presets::edit(|c| {
          let gone = c.remove(&name);
          if gone {
            c.save();
          }
          gone
        });
        self.say(if gone {
          "preset deleted".to_owned()
        } else {
          format!("no preset of yours named {}", presets::clean(&name))
        });
        self.rows_changed();
      }
      Cmd::Apply(name) => {
        if let Some(p) = PRESETS.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim())) {
          return self.apply_preset(p);
        }
        match presets::with(|c| c.find(&name).cloned()) {
          Some(c) => self.apply_custom(&c),
          None => self.say(format!("no preset named {}: /preset lists them", presets::clean(&name))),
        }
      }
    }
  }

  fn apply_custom(&mut self, c: &presets::Custom) {
    let moved = c.apply(&mut self.set);
    if moved.is_empty() {
      return self.say(format!("already matching {}", c.name));
    }
    for d in &moved {
      self.applied(d.fx);
    }
    self.set.save();
    self.say(format!("applied {}: {} change{}", c.name, moved.len(), if moved.len() == 1 { "" } else { "s" }));
  }

  pub(super) fn apply_custom_at(&mut self, i: usize) {
    if let Some(c) = presets::with(|c| c.all().get(i).cloned()) {
      self.apply_custom(&c);
    }
  }

  // x on the settings screen over one of your presets
  pub(super) fn delete_picked_preset(&mut self) {
    let Some(s) = &self.screen else { return };
    let (_, targets) = s.lines(&self.set);
    if let Some(Target::Custom(i)) = targets.get(s.st.at).copied()
      && let Some(name) = presets::with(|c| c.all().get(i).map(|p| p.name.clone()))
    {
      self.preset_cmd(Cmd::Delete(name));
    }
  }

  // the list changed under an open settings screen: keep the pick on a row that exists
  fn rows_changed(&mut self) {
    if let Some(s) = self.screen.as_mut() {
      let (lines, _) = s.lines(&self.set);
      s.st.settle(&lines);
    }
    self.bar_dirty = true;
    self.wake();
  }
}

#[cfg(test)]
mod tests {
  use super::super::screen::Page;
  use super::*;
  use crate::model::presets::Customs;
  use crate::model::settings::{EmoteSize, Settings};
  use crate::net::Net;
  use crate::ui::sheet::L;

  // own scratch folder per test thread: the file really is written and read back
  #[test]
  fn save_apply_and_delete_end_to_end() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    presets::edit(|c| *c = Customs::default());
    app.set.vi = true;
    app.set.emote_size = EmoteSize::X4;
    app.preset_cmd(Cmd::Save("My Build".into()));
    assert!(presets::with(|c| c.find("my build").is_some()));
    assert_eq!(Customs::load().all().len(), 1, "on disk");
    // back to the defaults, then apply it by name, any case
    app.set = Settings::default();
    app.preset_cmd(Cmd::Apply("MY BUILD".into()));
    assert!(app.set.vi && app.set.emote_size == EmoteSize::X4);
    // a built-in keeps its name; an unknown one changes nothing
    app.preset_cmd(Cmd::Apply("minimal".into()));
    assert!(!app.set.zebra);
    let before = app.set;
    app.preset_cmd(Cmd::Apply("nope".into()));
    assert!(app.set == before);
    // the settings screen lists it (active when it matches) with the save row; x on its row deletes it
    app.preset_cmd(Cmd::Apply("my build".into()));
    app.open_screen(Page::Settings);
    let (lines, targets) = app.screen.as_ref().unwrap().lines(&app.set);
    let at = targets.iter().position(|t| matches!(t, Target::Custom(0))).expect("its row");
    assert!(matches!(&lines[at], L::Row { a, pill: Some(true), .. } if a == "My Build"));
    assert!(targets.iter().any(|t| matches!(t, Target::SavePreset)));
    app.screen.as_mut().unwrap().st.at = at;
    app.delete_picked_preset();
    assert!(presets::with(|c| c.all().is_empty()));
    assert!(Customs::load().all().is_empty());
  }

  #[test]
  fn nothing_to_save_keeps_the_list() {
    let (net, _asked) = Net::tap();
    let mut app = App::new(net, vec!["forsen".into()], false);
    presets::edit(|c| *c = Customs::default());
    app.set = Settings::default();
    app.preset_cmd(Cmd::Save("empty".into()));
    assert!(presets::with(|c| c.all().is_empty()));
  }
}
