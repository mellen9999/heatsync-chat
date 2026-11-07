// the window system's own corners on unix (wayland and x11); the other platforms have their own file
use winit::event_loop::ActiveEventLoop;
use winit::platform::wayland::{ActiveEventLoopExtWayland, WindowAttributesExtWayland};
use winit::platform::x11::WindowAttributesExtX11;
use winit::window::WindowAttributes;

// the app id (wayland) and WM_CLASS (x11): what the .desktop file's icon is matched on
pub(super) fn named(attrs: WindowAttributes, id: &str) -> WindowAttributes {
  let attrs = WindowAttributesExtWayland::with_name(attrs, id, id);
  WindowAttributesExtX11::with_name(attrs, id, id)
}

// a focus request does nothing here, so a window nobody is looking at is made again where they are
pub(super) fn focus_by_reopening(el: &ActiveEventLoop) -> bool {
  el.is_wayland()
}
