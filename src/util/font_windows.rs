// the font lookup on windows: there is no tool to ask, and none is needed. the system's font list is DirectWrite's,
// and fontique (under the text engine) already reads it. so this only checks that the typed name could be a
// setting; whether the system has that family is answered where the text engine lives (app/font.rs →
// Text::installed_name), on the ui thread, with no file and no process
use super::{Found, askable};

pub fn find(name: &str) -> Result<Found, String> {
  askable(name).map(|n| Found { family: n.to_owned(), file: None })
}
