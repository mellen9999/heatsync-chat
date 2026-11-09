// families tried after the monospace for a glyph it lacks, on windows. Consolas has the letters, the arrows and
// the block shapes, but not the ui's symbols (the settings gear, hearts, ticks, the half-filled circle): there
// DirectWrite's own fallback only goes by script, and these are "common" to every script. these ship with
// windows 10 and 11: Segoe UI Symbol (the symbols, in text style), Segoe UI Emoji, Segoe UI, and Cambria Math
// for the math signs (⊘ ≤ ∘)
pub const BEHIND: &[&str] = &["Segoe UI Symbol", "Segoe UI Emoji", "Segoe UI", "Cambria Math"];

// the han and kana fonts that ship with windows, tried after the symbols. DirectWrite's own fallback finds none of
// them for text set in a named family (the ui's monospace), so chinese, japanese and korean would draw as boxes. the
// one that suits the ui language goes first: han is shared between the three, and each draws it in its own style
const SC: &[&str] = &["Microsoft YaHei", "SimSun", "Microsoft JhengHei", "Yu Gothic", "Malgun Gothic"];
const TC: &[&str] = &["Microsoft JhengHei", "Microsoft YaHei", "SimSun", "Yu Gothic", "Malgun Gothic"];
const JA: &[&str] = &["Yu Gothic", "Microsoft YaHei", "Microsoft JhengHei", "SimSun", "Malgun Gothic"];
const KO: &[&str] = &["Malgun Gothic", "Microsoft YaHei", "Microsoft JhengHei", "Yu Gothic", "SimSun"];

pub fn han(code: &str) -> &'static [&'static str] {
  match code {
    "zh_TW" => TC,
    "ja" => JA,
    "ko" => KO,
    _ => SC,
  }
}
