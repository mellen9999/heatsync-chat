// families tried after the monospace for a glyph it lacks, on windows. Consolas has the letters, the arrows and
// the block shapes, but not the ui's symbols (the settings gear, hearts, ticks, the half-filled circle): there
// DirectWrite's own fallback only goes by script, and these are "common" to every script. these ship with
// windows 10 and 11: Segoe UI Symbol (the symbols, in text style), Segoe UI Emoji, Segoe UI, and Cambria Math
// for the math signs (⊘ ≤ ∘)
pub const BEHIND: &[&str] = &["Segoe UI Symbol", "Segoe UI Emoji", "Segoe UI", "Cambria Math"];
