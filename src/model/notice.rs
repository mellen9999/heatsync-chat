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

use crate::i18n::{f, t};

pub fn plain(id: Option<&str>, text: &str) -> String {
  let secs = || number(text).map(|n| format!("{n}s"));
  let said = |s: &str| s.to_string();
  match id.unwrap_or("") {
    "slow_on" => match secs() {
      Some(s) => f("chat2.notice.slow_mode_is_on_one_line_every_secs", &[("secs", &s)]),
      None => said(t("chat2.notice.slow_mode_is_on")),
    },
    "slow_off" => said(t("chat2.notice.slow_mode_is_off")),
    "subs_on" => said(t("chat2.notice.subscribers_only_mode_is_on")),
    "subs_off" => said(t("chat2.notice.subscribers_only_mode_is_off")),
    "emote_only_on" => said(t("chat2.notice.emote_only_mode_is_on")),
    "emote_only_off" => said(t("chat2.notice.emote_only_mode_is_off")),
    "followers_on" => match follow_age(text) {
      Some(a) => f("chat2.notice.followers_only_mode_is_on_follow_for_age", &[("age", &a)]),
      None => said(t("chat2.notice.followers_only_mode_is_on")),
    },
    "followers_on_zero" => said(t("chat2.notice.followers_only_mode_is_on")),
    "followers_off" => said(t("chat2.notice.followers_only_mode_is_off")),
    "r9k_on" => said(t("chat2.notice.unique_chat_mode_is_on_no_repeated_lines")),
    "r9k_off" => said(t("chat2.notice.unique_chat_mode_is_off")),
    "msg_banned" => said(t("chat2.notice.you_are_banned_from_this_channel")),
    "msg_timedout" => match secs() {
      Some(s) => f("chat2.notice.you_are_timed_out_secs_left", &[("secs", &s)]),
      None => said(t("chat2.notice.you_are_timed_out")),
    },
    "msg_ratelimit" => said(t("chat2.notice.slow_down_you_re_sending_too_fast")),
    "msg_duplicate" => said(t("chat2.notice.that_line_was_just_sent_try_something_di")),
    "msg_slowmode" => match secs() {
      Some(s) => f("chat2.notice.slow_mode_is_on_wait_secs_to_send_again", &[("secs", &s)]),
      None => said(t("chat2.notice.slow_mode_is_on_wait_a_moment_to_send_ag")),
    },
    "msg_subsonly" => said(t("chat2.notice.only_subscribers_can_chat_here")),
    "msg_followersonly" => said(t("chat2.notice.only_followers_can_chat_here_follow_the_")),
    "msg_followersonly_followed" => said(t("chat2.notice.you_haven_t_followed_long_enough_to_chat")),
    "msg_followersonly_zero" => said(t("chat2.notice.only_followers_can_chat_here_follow_the_")),
    "msg_emoteonly" => said(t("chat2.notice.emote_only_mode_is_on_send_only_emotes")),
    "msg_channel_suspended" => said(t("chat2.notice.this_channel_is_closed")),
    "msg_channel_blocked" => said(t("chat2.notice.this_channel_won_t_let_you_chat")),
    "msg_suspended" => said(t("chat2.notice.your_twitch_account_can_t_chat_right_now")),
    "msg_verified_email" => said(t("chat2.notice.verify_your_email_on_twitch_to_chat_here")),
    "msg_requires_verified_phone_number" => said(t("chat2.notice.add_a_phone_number_on_twitch_to_chat_her")),
    "msg_room_not_found" => said(t("chat2.notice.that_channel_doesn_t_exist")),
    "msg_rejected" | "msg_rejected_mandatory" => said(t("chat2.notice.that_line_was_held_back_by_the_channel_s")),
    "msg_bad_characters" => said(t("chat2.notice.that_line_has_characters_twitch_won_t_se")),
    "unrecognized_cmd" => said(t("chat2.notice.twitch_doesn_t_know_that_command")),
    "no_permission" => said(t("chat2.notice.you_can_t_do_that_here")),
    "bad_ban_self" | "bad_timeout_self" => said(t("chat2.notice.you_can_t_do_that_to_yourself")),
    "bad_ban_broadcaster" | "bad_timeout_broadcaster" => said(t("chat2.notice.you_can_t_do_that_to_the_streamer")),
    "bad_ban_mod" | "bad_timeout_mod" => said(t("chat2.notice.you_can_t_do_that_to_a_mod")),
    "already_banned" => said(t("chat2.notice.they_re_already_banned")),
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
