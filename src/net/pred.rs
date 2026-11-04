// a channel's prediction from twitch's public web gql, no login, 5s at most (ext fetchPrediction)
use super::{NetEvent, Out};

pub async fn fetch(http: reqwest::Client, chan: Box<str>, out: Out) {
  // a twitch login: nothing that could bend the query (it is a variable anyway)
  let ok = !chan.is_empty() && chan.len() <= 25 && chan.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
  let res = if ok { ask(&http, &chan).await } else { Err(()) };
  out.send(NetEvent::Pred { chan, res });
}

async fn ask(http: &reqwest::Client, chan: &str) -> Result<Option<crate::model::pred::Pred>, ()> {
  let req = http
    .post(crate::badges::fetch::GQL)
    .header("Client-Id", crate::badges::fetch::GQL_CLIENT_ID)
    .timeout(std::time::Duration::from_secs(5))
    .body(crate::model::pred::query(chan).to_string());
  let (status, v) = super::social::call(req).await;
  if status != 200 {
    return Err(());
  }
  crate::model::pred::parse(&v, crate::util::time::now_ms()).ok_or(())
}
