// playing a wav on windows: PlaySound from the bytes in memory (winmm; no process, no file). played in sync on
// a thread of its own, so the buffer lives exactly as long as the sound and the ui thread never waits. a new
// sound cuts the one still playing, which is what picking a sound in the settings should do
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};

pub fn play(bytes: impl FnOnce() -> Vec<u8> + Send + 'static) {
  let _ = std::thread::Builder::new().name("sound".into()).spawn(move || {
    let wav = bytes();
    // SAFETY: SND_MEMORY reads the wav from this pointer, and SND_SYNC does not return before it is done
    let ok = unsafe { PlaySoundW(wav.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_SYNC | SND_NODEFAULT) };
    if ok == 0 {
      eprintln!("the mention sound didn't play (is there a sound device?)");
    }
  });
}
