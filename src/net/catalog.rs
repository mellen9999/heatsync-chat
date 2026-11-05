// tab-complete's catalog search (ext mcSearch7tvApi / mcSearchFfzApi): one page from each provider,
// asked together, anonymous, nothing of yours sent but the word typed. answers are bounded and checked
// in `model::catalog` before the ui sees them
use std::time::Duration;

use serde_json::{Value, json};

use super::{NetEvent, Out};
use crate::model::catalog::{PAGE_FFZ, PAGE_STV, Page, parse_7tv, parse_ffz, parse_hs};

// a page of 200 ffz rows with art urls is a few hundred KB
const MAX_BODY: usize = 1 << 20;
const TIMEOUT: Duration = Duration::from_secs(8);
const STV: &str = "https://api.7tv.app/v4/gql";
const QUERY: &str = "query SearchEmotes($query: String!, $page: Int!, $perPage: Int!) { emotes { search(query: $query, sort: { sortBy: TOP_ALL_TIME, order: DESCENDING }, page: $page, perPage: $perPage) { totalCount items { id defaultName flags { animated defaultZeroWidth } } } } }";

fn ffz_url(q: &str, page: u32) -> String {
  format!(
    "https://api.frankerfacez.com/v1/emotes?q={}&sort=count-desc&per_page={PAGE_FFZ}&page={page}",
    crate::util::path_seg(q)
  )
}

fn stv_body(q: &str, page: u32) -> String {
  json!({"operationName": "SearchEmotes", "query": QUERY, "variables": {"query": q, "page": page, "perPage": PAGE_STV}})
    .to_string()
}

async fn body(r: Result<reqwest::Response, reqwest::Error>) -> Option<Value> {
  serde_json::from_slice(&super::http::capped(r, MAX_BODY).await.ok()?).ok()
}

pub async fn run(http: reqwest::Client, q: Box<str>, epoch: u32, page: u32, ask: (bool, bool), out: Out) {
  let valid = crate::model::catalog::eligible(&q) && (1..=1000).contains(&page);
  let stv = async {
    if !valid || !ask.0 {
      return None;
    }
    let r =
      http.post(STV).header("content-type", "application/json").timeout(TIMEOUT).body(stv_body(&q, page)).send().await;
    body(r).await.map(|v| parse_7tv(&v))
  };
  let ffz = async {
    if !valid || !ask.1 {
      return None;
    }
    body(http.get(ffz_url(&q, page)).timeout(TIMEOUT).send().await).await.map(|v| parse_ffz(&v))
  };
  let (stv, ffz): (Option<Page>, Option<Page>) = tokio::join!(stv, ffz);
  out.send(NetEvent::Catalog { epoch, stv, ffz });
}

// the picker's search (ext mcSearch7tvApi / mcSearchFfzApi / mcSearchHsApi): the same pages as tab-complete's
// plus heatsync's own directory, which has one page and no login (a public route; no token goes with it).
// `q` is already `picker_query`-clean; checked again here because this is what leaves the machine
pub async fn search(http: reqwest::Client, q: Box<str>, epoch: u32, page: u32, ask: [bool; 3], out: Out) {
  let valid = crate::model::catalog::picker_query(&q).is_some_and(|c| *c == *q) && (1..=1000).contains(&page);
  let stv = async {
    if !valid || !ask[0] {
      return None;
    }
    let r =
      http.post(STV).header("content-type", "application/json").timeout(TIMEOUT).body(stv_body(&q, page)).send().await;
    body(r).await.map(|v| parse_7tv(&v))
  };
  let ffz = async {
    if !valid || !ask[1] {
      return None;
    }
    body(http.get(ffz_url(&q, page)).timeout(TIMEOUT).send().await).await.map(|v| parse_ffz(&v))
  };
  let hs = async {
    if !valid || !ask[2] || page != 1 {
      return None;
    }
    body(http.get(hs_url(&q)).timeout(TIMEOUT).send().await).await.map(|v| parse_hs(&v))
  };
  let (stv, ffz, hs) = tokio::join!(stv, ffz, hs);
  out.send(NetEvent::PickerSearch { epoch, pages: [stv, ffz, hs] });
}

fn hs_url(q: &str) -> String {
  format!("{}/api/emote-search?q={}&p=hs", super::ORIGIN, crate::util::path_seg(q))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn requests_carry_the_word_encoded_and_the_ext_page_sizes() {
    assert_eq!(
      ffz_url("a&b c", 2),
      "https://api.frankerfacez.com/v1/emotes?q=a%26b%20c&sort=count-desc&per_page=200&page=2"
    );
    let b: Value = serde_json::from_str(&stv_body("sad\"x", 3)).unwrap();
    assert_eq!(b["variables"], json!({"query": "sad\"x", "page": 3, "perPage": 60}));
    assert_eq!(b["operationName"], "SearchEmotes");
    assert_eq!(hs_url("a:b.c"), "https://heatsync.org/api/emote-search?q=a%3Ab.c&p=hs");
  }
}
