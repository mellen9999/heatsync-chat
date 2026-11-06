// twitch's NOTICE lines in our own plain words. twitch sends a msg-id with each (docs: irc#notice); the ones
// people actually meet get our lowercase copy, anything else is twitch's text lowercased without its last period.
// the numbers come off the wire, so they're read defensively
fn number(s: &str) -> Option<u32> {
  let digits: String = s.chars().skip_while(|c| !c.is_ascii_digit()).take_while(char::is_ascii_digit).take(9).collect();
  digits.parse().ok()
}

// "This room is now in 30 minutes followers-only mode." -> "30 minutes"
fn follow_age(text: &str) -> Option<&str> {
  let rest = text.split_once("now in ")?.1;
  let age = rest.split_once(" followers-only")?.0.trim();
  (!age.is_empty() && age.len() <= 40 && age.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ')).then_some(age)
}

fn fallback(text: &str) -> String {
  text.trim().to_lowercase().trim_end_matches('.').trim_end().replace('’', "'")
}

pub fn plain(id: Option<&str>, text: &str) -> String {
  let secs = || number(text).map(|n| format!("{n}s"));
  let said = |s: &str| s.to_string();
  match id.unwrap_or("") {
    "slow_on" => match secs() {
      Some(s) => format!("slow mode is on · one line every {s}"),
      None => said("slow mode is on"),
    },
    "slow_off" => said("slow mode is off"),
    "subs_on" => said("subscribers-only mode is on"),
    "subs_off" => said("subscribers-only mode is off"),
    "emote_only_on" => said("emote-only mode is on"),
    "emote_only_off" => said("emote-only mode is off"),
    "followers_on" => match follow_age(text) {
      Some(a) => format!("followers-only mode is on · follow for {a} to chat"),
      None => said("followers-only mode is on"),
    },
    "followers_on_zero" => said("followers-only mode is on"),
    "followers_off" => said("followers-only mode is off"),
    "r9k_on" => said("unique-chat mode is on · no repeated lines"),
    "r9k_off" => said("unique-chat mode is off"),
    "msg_banned" => said("you are banned from this channel"),
    "msg_timedout" => match secs() {
      Some(s) => format!("you are timed out · {s} left"),
      None => said("you are timed out"),
    },
    "msg_ratelimit" => said("slow down, you're sending too fast"),
    "msg_duplicate" => said("that line was just sent, try something different"),
    "msg_slowmode" => match secs() {
      Some(s) => format!("slow mode is on · wait {s} to send again"),
      None => said("slow mode is on · wait a moment to send again"),
    },
    "msg_subsonly" => said("only subscribers can chat here"),
    "msg_followersonly" => said("only followers can chat here · follow the channel first"),
    "msg_followersonly_followed" => said("you haven't followed long enough to chat here"),
    "msg_followersonly_zero" => said("only followers can chat here · follow the channel first"),
    "msg_emoteonly" => said("emote-only mode is on · send only emotes"),
    "msg_channel_suspended" => said("this channel is closed"),
    "msg_channel_blocked" => said("this channel won't let you chat"),
    "msg_suspended" => said("your twitch account can't chat right now"),
    "msg_verified_email" => said("verify your email on twitch to chat here"),
    "msg_requires_verified_phone_number" => said("add a phone number on twitch to chat here"),
    "msg_room_not_found" => said("that channel doesn't exist"),
    "msg_rejected" | "msg_rejected_mandatory" => said("that line was held back by the channel's rules"),
    "msg_bad_characters" => said("that line has characters twitch won't send"),
    "unrecognized_cmd" => said("twitch doesn't know that command"),
    "no_permission" => said("you can't do that here"),
    "bad_ban_self" | "bad_timeout_self" => said("you can't do that to yourself"),
    "bad_ban_broadcaster" | "bad_timeout_broadcaster" => said("you can't do that to the streamer"),
    "bad_ban_mod" | "bad_timeout_mod" => said("you can't do that to a mod"),
    "already_banned" => said("they're already banned"),
    _ => fallback(text),
  }
}

#[cfg(test)]
mod tests {
  use super::plain;

  #[test]
  fn known_ids_get_our_words() {
    assert_eq!(
      plain(Some("slow_on"), "This room is now in slow mode. You may send messages every 5 seconds."),
      "slow mode is on · one line every 5s"
    );
    assert_eq!(plain(Some("slow_off"), "This room is no longer in slow mode."), "slow mode is off");
    assert_eq!(plain(Some("subs_on"), "x"), "subscribers-only mode is on");
    assert_eq!(plain(Some("r9k_on"), "x"), "unique-chat mode is on · no repeated lines");
    assert_eq!(
      plain(Some("msg_banned"), "You are permanently banned from talking in c."),
      "you are banned from this channel"
    );
    assert_eq!(plain(Some("msg_timedout"), "You are timed out for 577 more seconds."), "you are timed out · 577s left");
    assert_eq!(
      plain(Some("msg_ratelimit"), "Your message was not sent because you are sending messages too quickly."),
      "slow down, you're sending too fast"
    );
    assert_eq!(
      plain(
        Some("msg_slowmode"),
        "This room is in slow mode and you are sending messages too quickly. You will be able to talk again in 12 seconds."
      ),
      "slow mode is on · wait 12s to send again"
    );
  }

  #[test]
  fn followers_on_names_how_long() {
    assert_eq!(
      plain(Some("followers_on"), "This room is now in 30 minutes followers-only mode."),
      "followers-only mode is on · follow for 30 minutes to chat"
    );
    assert_eq!(plain(Some("followers_on"), "This room is now in followers-only mode."), "followers-only mode is on");
    assert_eq!(
      plain(Some("followers_on_zero"), "This room is now in followers-only mode."),
      "followers-only mode is on"
    );
  }

  #[test]
  fn unknown_ids_are_twitchs_text_lowercased_without_the_period() {
    assert_eq!(plain(Some("something_new"), "Hello There. Nice."), "hello there. nice");
    assert_eq!(plain(None, "Login authentication failed"), "login authentication failed");
    assert_eq!(plain(Some("slow_on"), "no digits"), "slow mode is on");
  }
}
