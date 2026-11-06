// your own slash commands at the message box (model/usercmd.rs holds the file and the placeholders). the
// expansion happens once, in `send`, after every built-in command had its look: a built-in always wins a
// name. the expanded line then goes the normal way (built-in command or chat line, with all its checks).
// it can't loop: a result that starts another of your commands is refused, so the second pass never
// finds a command of yours to expand
use super::App;
use crate::model::message::Platform;
use crate::model::usercmd::{self, Ctx};
use crate::model::{chan, chantools, filtercmd, modcmd, modes, modsuite, notes, postcmd, settings, slash, whisper};

// does a built-in answer to `/name`? (the listed ones, and the aliases that are not listed)
pub(super) fn builtin(name: &str) -> bool {
  let slash_name = format!("/{name}");
  if slash::all().iter().any(|c| c.name == slash_name) {
    return true;
  }
  [slash_name.clone(), format!("{slash_name} a"), format!("{slash_name} a b")].iter().any(|t| {
    settings::local_cmd(t).is_some()
      || chan::command(t).is_some()
      || filtercmd::command(t).is_some()
      || notes::command(t).is_some()
      || modes::command(t).is_some()
      || modsuite::command(t).is_some()
      || chantools::command(t).is_some()
      || modcmd::command(t).is_some()
      || postcmd::command(t).is_some()
      || whisper::command(t).is_some()
  })
}

impl App {
  // the text a typed line of yours becomes. None = not one of yours (or a built-in's name); Some(Err) = said why not
  fn ucmd_text(&self) -> Option<Result<String, String>> {
    let (name, input) = usercmd::split(&self.line.text)?;
    let template = self.ucmds.get(&name)?;
    if builtin(&name) {
      return None;
    }
    let tab = self.tabs.get(self.active);
    let platform = self.target().map_or(Platform::Twitch, |t| t.0);
    let ctx =
      Ctx { channel: tab.map_or("", |t| &*t.view.channel), user: self.me.as_ref().and_then(|m| m.name_on(platform)) };
    let text = match usercmd::expand(template, &name, input, &ctx) {
      Ok(t) => t,
      Err(e) => return Some(Err(e)),
    };
    // a command of yours can't run another of yours: that is the whole loop guard
    if let Some((again, _)) = usercmd::split(&text)
      && self.ucmds.get(&again).is_some()
      && !builtin(&again)
    {
      return Some(Err(format!("/{name} can't run /{again}: a command of yours can only run built-in ones")));
    }
    Some(Ok(text))
  }

  // Enter on a line that is one of your commands. true = it took the send (it said why, or sent the expansion)
  pub(super) fn ucmd_send(&mut self) -> bool {
    let Some(r) = self.ucmd_text() else { return false };
    match r {
      Err(e) => self.say(e),
      Ok(text) => {
        let typed = self.line.text.clone();
        self.line.set_text(text.clone());
        self.line.cursor = text.len();
        self.send();
        // refused on the way (locked out, no network): what you typed stays, not the expansion
        if self.line.text == text {
          self.line.set_text(typed);
          self.line.cursor = self.line.text.len();
        }
      }
    }
    true
  }

  // the list screen's add: kept and saved, with a word about a name a built-in has. Err = kept in the box
  pub(super) fn ucmd_add(&mut self, typed: &str) -> Result<String, String> {
    let name = self.ucmds.add(typed).map_err(str::to_owned)?;
    self.ucmds.save();
    Ok(if builtin(&name) {
      format!("saved, but /{name} is a built-in command and always wins: yours never runs. use another name")
    } else {
      format!("added /{name}")
    })
  }

  pub(super) fn ucmd_remove(&mut self, name: &str) -> String {
    if self.ucmds.remove(name) {
      self.ucmds.save();
    }
    format!("removed /{name}")
  }
}
