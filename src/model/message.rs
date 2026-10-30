// one chat line. kept flat: nick + body, colors resolved at layout time
#[derive(Clone, Debug)]
pub struct Message {
  pub ts: i64,
  pub nick: Box<str>,
  pub color: u32,
  pub body: Box<str>,
}
