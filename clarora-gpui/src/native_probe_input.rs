//! Native pointer events for isolated hidden-window diagnostics.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum MouseEvent {
    Down(Point<Pixels>),
    Drag(Point<Pixels>),
    Up(Point<Pixels>),
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct AppKitPoint {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
// AppKit's CGPoint consists of two CGFloat values on supported macOS targets.
unsafe impl objc2::encode::Encode for AppKitPoint {
    const ENCODING: objc2::encode::Encoding =
        objc2::encode::Encoding::Struct("CGPoint", &[<f64 as objc2::encode::Encode>::ENCODING; 2]);
}

#[cfg(target_os = "macos")]
pub(super) fn send(
    window: &Window,
    cx: &mut Context<AppView>,
    events: Vec<MouseEvent>,
) -> Result<()> {
    use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject, sel};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let native_window = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow::anyhow!("native diagnostic window handle: {error:?}"))?;
    let RawWindowHandle::AppKit(native) = native_window.as_raw() else {
        anyhow::bail!("native pointer diagnostic requires an AppKit window");
    };
    let view = native.ns_view.as_ptr() as *mut AnyObject;
    let panel: *mut AnyObject = unsafe { msg_send![view, window] };
    ensure!(!panel.is_null(), "native pointer diagnostic has no window");
    let number: isize = unsafe { msg_send![panel, windowNumber] };
    let height = window.viewport_size().height;
    cx.defer(move |_| {
        let event_class = AnyClass::get(c"NSEvent").expect("NSEvent class");
        for (index, event) in events.into_iter().enumerate() {
            let (kind, position, selector) = match event {
                MouseEvent::Down(position) => (1usize, position, sel!(mouseDown:)),
                MouseEvent::Drag(position) => (6usize, position, sel!(mouseDragged:)),
                MouseEvent::Up(position) => (2usize, position, sel!(mouseUp:)),
            };
            let position = AppKitPoint {
                x: f64::from(f32::from(position.x)),
                y: f64::from(f32::from(height - position.y)),
            };
            let event: *mut AnyObject = unsafe {
                msg_send![event_class,
                    mouseEventWithType: kind,
                    location: position,
                    modifierFlags: 0usize,
                    timestamp: 0f64,
                    windowNumber: number,
                    context: std::ptr::null_mut::<AnyObject>(),
                    eventNumber: 0isize,
                    clickCount: 1isize,
                    pressure: 1f32
                ]
            };
            assert!(!event.is_null(), "create native pointer event");
            // Deliver after this GPUI update releases the app borrow.
            unsafe {
                let _: () = msg_send![
                    view, performSelector: selector,
                    withObject: event, afterDelay: (index + 1) as f64 * 0.02
                ];
            }
        }
    });
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn send(_: &Window, _: &mut Context<AppView>, _: Vec<MouseEvent>) -> Result<()> {
    anyhow::bail!("native pointer diagnostic requires macOS");
}
