//! A native exit path that works independently of the focused GPUI window.
use super::{EXIT_SHORTCUT, Interruption, native};
use anyhow::{Context, Result, ensure};
use block2::RcBlock;
use objc2::{MainThreadMarker, class, msg_send, rc::Retained, runtime::AnyObject};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::{NonNull, null_mut};

const SIGNATURE: u32 = u32::from_be_bytes(*b"Clra");
const KEYBOARD_EVENT: u32 = u32::from_be_bytes(*b"keyb");
const HOT_KEY_PRESSED: u32 = 5;
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const HOT_KEY_ID: u32 = u32::from_be_bytes(*b"hkid");
const EVENT_NOT_HANDLED: i32 = -9874;
const ESCAPE_KEY: u32 = 0x35;
const CONTROL_SHIFT: u32 = (1 << 12) | (1 << 9);
const EXCLUSIVE: u32 = 1;

#[repr(C)]
struct EventTypeSpec {
    class: u32,
    kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        callback: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
        count: usize,
        types: *const EventTypeSpec,
        data: *mut c_void,
        handler: *mut *mut c_void,
    ) -> i32;
    fn RemoveEventHandler(handler: *mut c_void) -> i32;
    fn RegisterEventHotKey(
        key: u32,
        modifiers: u32,
        id: EventHotKeyId,
        target: *mut c_void,
        options: u32,
        hot_key: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hot_key: *mut c_void) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: usize,
        actual_size: *mut usize,
        data: *mut c_void,
    ) -> i32;
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSApplicationDidResignActiveNotification: *const AnyObject;
}

#[derive(Clone, Copy)]
struct Session {
    id: u32,
    original_options: usize,
    interruption: Option<Interruption>,
}

thread_local! {
    // The Carbon callback carries a numeric ID, so a queued event cannot access
    // a freed Rust object or accidentally apply to the next focus session.
    static SESSION: Cell<Option<Session>> = const { Cell::new(None) };
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
}

fn release(id: u32, interruption: Interruption) -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    let original = SESSION.with(|slot| {
        let mut session = slot.get().filter(|session| session.id == id)?;
        if session.interruption.is_some() {
            return None;
        }
        session.interruption = Some(interruption);
        slot.set(Some(session));
        Some(session.original_options)
    });
    if let Some(original) = original {
        // Restore before returning to AppKit. Window and timer cleanup happen
        // on the next AppView tick, outside the native callback.
        native::set(original);
        true
    } else {
        false
    }
}

unsafe extern "C" fn hot_key_pressed(_: *mut c_void, event: *mut c_void, _: *mut c_void) -> i32 {
    let mut id = EventHotKeyId {
        signature: 0,
        id: 0,
    };
    // Carbon owns this EventRef for the duration of the callback.
    let status = unsafe {
        GetEventParameter(
            event,
            DIRECT_OBJECT,
            HOT_KEY_ID,
            null_mut(),
            size_of::<EventHotKeyId>(),
            null_mut(),
            (&mut id as *mut EventHotKeyId).cast(),
        )
    };
    if status == 0 && id.signature == SIGNATURE && release(id.id, Interruption::Shortcut) {
        0
    } else {
        EVENT_NOT_HANDLED
    }
}

struct Shortcut(NonNull<c_void>);

impl Shortcut {
    fn register(id: u32, target: NonNull<c_void>) -> Result<Self> {
        let mut hot_key = null_mut();
        let status = unsafe {
            RegisterEventHotKey(
                ESCAPE_KEY,
                CONTROL_SHIFT,
                EventHotKeyId {
                    signature: SIGNATURE,
                    id,
                },
                target.as_ptr(),
                EXCLUSIVE,
                &mut hot_key,
            )
        };
        ensure!(
            status == 0,
            "无法注册专注退出快捷键 {EXIT_SHORTCUT}（系统错误 {status}）。请检查是否已被占用"
        );
        Ok(Self(
            NonNull::new(hot_key).context("系统未返回专注退出快捷键")?,
        ))
    }
}

impl Drop for Shortcut {
    fn drop(&mut self) {
        unsafe { UnregisterEventHotKey(self.0.as_ptr()) };
    }
}

pub(super) struct Recovery {
    id: u32,
    handler: NonNull<c_void>,
    observer: Option<(Retained<AnyObject>, Retained<AnyObject>)>,
    _shortcut: Shortcut,
    _main_thread: MainThreadMarker,
}

impl Recovery {
    pub(super) fn install(original_options: usize) -> Result<Self> {
        let main_thread = MainThreadMarker::new().context("专注限制需要在主线程开启")?;
        ensure!(SESSION.with(Cell::get).is_none(), "电脑专注已开启");
        let id = NEXT_ID.with(|next| {
            let id = next.get();
            next.set(id.wrapping_add(1));
            id
        });
        let target = NonNull::new(unsafe { GetApplicationEventTarget() })
            .context("系统未提供专注退出快捷键的事件目标")?;
        let shortcut = Shortcut::register(id, target)?;
        let event = EventTypeSpec {
            class: KEYBOARD_EVENT,
            kind: HOT_KEY_PRESSED,
        };
        let mut handler = null_mut();
        let status = unsafe {
            InstallEventHandler(
                target.as_ptr(),
                hot_key_pressed,
                1,
                &event,
                null_mut(),
                &mut handler,
            )
        };
        ensure!(status == 0, "无法监听专注退出快捷键（系统错误 {status}）");
        let mut recovery = Self {
            id,
            handler: NonNull::new(handler).context("系统未返回专注退出事件处理器")?,
            observer: None,
            _shortcut: shortcut,
            _main_thread: main_thread,
        };
        let block = RcBlock::new(move |_: *mut AnyObject| {
            release(id, Interruption::Deactivated);
        });
        // AppKit delivers this app-specific notification on the main thread.
        // With a nil queue the observer restores options during delivery.
        let (center, observer) = unsafe {
            let center: Retained<AnyObject> =
                msg_send![class!(NSNotificationCenter), defaultCenter];
            let observer: Option<Retained<AnyObject>> = msg_send![
                &*center,
                addObserverForName: NSApplicationDidResignActiveNotification,
                object: native::application(),
                queue: null_mut::<AnyObject>(),
                usingBlock: &*block
            ];
            (center, observer.context("无法监听 Clarora 的前台状态")?)
        };
        recovery.observer = Some((center, observer));
        SESSION.with(|slot| {
            slot.set(Some(Session {
                id,
                original_options,
                interruption: None,
            }))
        });
        Ok(recovery)
    }

    pub(super) fn interruption(&self) -> Option<Interruption> {
        SESSION.with(|slot| {
            slot.get()
                .filter(|session| session.id == self.id)?
                .interruption
        })
    }
}

impl Drop for Recovery {
    fn drop(&mut self) {
        SESSION.with(|slot| {
            if slot.get().is_some_and(|session| session.id == self.id) {
                slot.set(None);
            }
        });
        unsafe {
            if let Some((center, observer)) = &self.observer {
                let _: () = msg_send![&**center, removeObserver: &**observer];
            }
            RemoveEventHandler(self.handler.as_ptr());
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) mod probe {
    use super::*;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn CreateEvent(
            allocator: *const c_void,
            class: u32,
            kind: u32,
            time: f64,
            attributes: u32,
            event: *mut *mut c_void,
        ) -> i32;
        fn SetEventParameter(
            event: *mut c_void,
            name: u32,
            kind: u32,
            size: usize,
            data: *const c_void,
        ) -> i32;
        fn SendEventToEventTarget(event: *mut c_void, target: *mut c_void) -> i32;
        fn ReleaseEvent(event: *mut c_void);
    }

    pub(crate) fn id() -> Result<u32> {
        SESSION
            .with(Cell::get)
            .map(|session| session.id)
            .context("no registered focus session")
    }

    pub(crate) fn with_reserved_shortcut<T>(run: impl FnOnce() -> Result<T>) -> Result<T> {
        ensure!(
            !native::is_active(),
            "shortcut probe requires an inactive app"
        );
        let target =
            NonNull::new(unsafe { GetApplicationEventTarget() }).context("event target")?;
        let _key = Shortcut::register(0, target)?;
        run()
    }

    pub(crate) fn dispatch_shortcut(id: u32) -> Result<()> {
        ensure!(
            !native::is_active(),
            "shortcut probe requires an inactive app"
        );
        let mut event = null_mut();
        let status = unsafe {
            CreateEvent(
                std::ptr::null(),
                KEYBOARD_EVENT,
                HOT_KEY_PRESSED,
                0.,
                0,
                &mut event,
            )
        };
        ensure!(
            status == 0 && !event.is_null(),
            "create hot key fixture: {status}"
        );
        let id = EventHotKeyId {
            signature: SIGNATURE,
            id,
        };
        let status = unsafe {
            let status = SetEventParameter(
                event,
                DIRECT_OBJECT,
                HOT_KEY_ID,
                size_of::<EventHotKeyId>(),
                (&id as *const EventHotKeyId).cast(),
            );
            if status == 0 {
                // Send to this process only. No synthetic system keystrokes.
                SendEventToEventTarget(event, GetApplicationEventTarget());
            }
            ReleaseEvent(event);
            status
        };
        ensure!(status == 0, "set hot key fixture parameter: {status}");
        Ok(())
    }
}
