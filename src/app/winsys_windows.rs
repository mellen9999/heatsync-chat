// the window system's own corners on windows; the other platforms have their own file
use winit::event_loop::ActiveEventLoop;
use winit::window::{Theme, WindowAttributes};

// dark only, so the title bar is too. windows has no app id to name: the exe carries its own
pub(super) fn named(attrs: WindowAttributes, _id: &str) -> WindowAttributes {
  attrs.with_theme(Some(Theme::Dark))
}

// windows lets the app that was just started ask for focus (and flashes the taskbar button when it may not),
// so the window we have is the window we use
pub(super) fn focus_by_reopening(_: &ActiveEventLoop) -> bool {
  false
}
