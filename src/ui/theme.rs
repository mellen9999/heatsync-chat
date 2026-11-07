// ansi 0-7 + the three sanctioned grays/brand tokens. nothing else paints
pub const BLACK: u32 = 0x000000;
pub const RED: u32 = 0xff0000;
pub const GREEN: u32 = 0x00ff00;
pub const YELLOW: u32 = 0xffff00;
pub const BLUE: u32 = 0x8888ff;
pub const MAGENTA: u32 = 0xff00ff;
pub const CYAN: u32 = 0x00ffff;
pub const WHITE: u32 = 0xffffff;
pub const ZEBRA: u32 = 0x444444;
pub const DIM: u32 = 0x808080;
// dim text on a zebra stripe, where #808080 sinks into the gray (the reply line)
pub const DIM_STRIPE: u32 = 0xbcbcbc;
pub const HEATSYNC: u32 = 0xff8700;
pub const TWITCH: u32 = 0xc8a8ff;
// the tile behind a channel's own ffz mod badge (ext HS_MOD_GREEN): ffz's art is a glyph on transparency
pub const MOD_BADGE: u32 = 0x00ad03;
// a line that pings you (ext --hs-mention-bg): a solid fill like the rest below, so it never takes a stripe
pub const MENTION_BG: u32 = 0x5f0000;

// filled rows (heatsync shared/brand/tokens.json `fills`, `labels`, `row`): a notification row is a solid fill
// from the dim set, the dim twin of what it means. body text on one is white; the label keeps `LABEL_*`
pub const FILL_OLIVE: u32 = 0x808000; // stream events
pub const FILL_PURPLE: u32 = 0x800080; // new people, first messages
pub const FILL_NAVY: u32 = 0x000080; // support: subs, gifts, cheers
pub const FILL_TEAL: u32 = 0x008080; // channel points + the broadcaster's voice
pub const FILL_MAROON: u32 = 0x800000; // a rule's red
pub const FILL_GREEN: u32 = 0x008000; // a rule's green
pub const LABEL_OLIVE: u32 = YELLOW;
pub const LABEL_PURPLE: u32 = WHITE;
pub const LABEL_NAVY: u32 = BLUE;
pub const LABEL_TEAL: u32 = CYAN;
pub const LABEL_RED: u32 = WHITE; // the mention fill and every rule fill
// the black plate a sender's name sits on, wider than the name by this much each side (tokens row.cutPadX)
pub const CUT_PAD_X: u16 = 4;

// a rule's colour (one of the eight) -> its row fill (tokens ruleFill): the dim twin, white = the zebra gray
pub fn rule_fill(ink: u32) -> u32 {
  match ink {
    RED => FILL_MAROON,
    GREEN => FILL_GREEN,
    YELLOW => FILL_OLIVE,
    BLUE => FILL_NAVY,
    MAGENTA => FILL_PURPLE,
    CYAN => FILL_TEAL,
    _ => ZEBRA,
  }
}

pub const FONT_PX: f32 = 15.0;
pub const LINE_PX: f32 = 20.0;
pub const PAD_X: f32 = 4.0;

#[cfg(test)]
mod tests {
  use super::*;

  // the numbers above against heatsync's tokens.json when this machine has it (HSC_TOKENS, else the usual
  // checkout); silently skipped elsewhere
  #[test]
  fn fills_match_the_brand_tokens() {
    let path = std::env::var("HSC_TOKENS")
      .ok()
      .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/projects/heatsync/shared/brand/tokens.json")));
    let Some(raw) = path.and_then(|p| std::fs::read_to_string(p).ok()) else { return };
    let t: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let hex = |v: &serde_json::Value| u32::from_str_radix(v.as_str().unwrap().trim_start_matches('#'), 16).unwrap();
    let f = &t["fills"];
    for (k, mine) in [
      ("olive", FILL_OLIVE),
      ("purple", FILL_PURPLE),
      ("navy", FILL_NAVY),
      ("teal", FILL_TEAL),
      ("maroon", FILL_MAROON),
      ("green", FILL_GREEN),
    ] {
      assert_eq!(hex(&f[k]), mine, "fill {k}");
    }
    assert_eq!(hex(&t["mention"]["bg"]), MENTION_BG);
    assert_eq!(hex(&t["zebra"]), ZEBRA);
    let label = |n: &str| hex(&t["palette"][t["labels"][n].as_str().unwrap()]);
    for (k, mine) in [("olive", LABEL_OLIVE), ("purple", LABEL_PURPLE), ("navy", LABEL_NAVY), ("teal", LABEL_TEAL)] {
      assert_eq!(label(k), mine, "label {k}");
    }
    assert_eq!(label("red"), LABEL_RED);
    assert_eq!(t["row"]["cutPadX"].as_str(), Some("4px"));
    for (name, ink) in [
      ("red", RED),
      ("green", GREEN),
      ("yellow", YELLOW),
      ("blue", BLUE),
      ("magenta", MAGENTA),
      ("cyan", CYAN),
      ("white", WHITE),
    ] {
      let want = match t["ruleFill"][name].as_str().unwrap() {
        "zebra" => ZEBRA,
        fill => hex(&f[fill]),
      };
      assert_eq!(rule_fill(ink), want, "rule {name}");
    }
  }
}
