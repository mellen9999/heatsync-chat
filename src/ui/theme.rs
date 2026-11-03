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
// a line that pings you (ext --hs-mention-bg); on a zebra row the lighter red, never the gray
pub const MENTION_BG: u32 = 0x5f0000;
pub const MENTION_STRIPE: u32 = 0x870000;

pub const FONT_PX: f32 = 15.0;
pub const LINE_PX: f32 = 20.0;
pub const PAD_X: f32 = 4.0;
