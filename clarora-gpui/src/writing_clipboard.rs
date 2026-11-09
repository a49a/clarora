//! GPUI exposes plain text; this field also reads the platform HTML clipboard flavor.
#[cfg(any(target_os = "macos", windows))]
use clarora_gpui::writing_format::MAX_HTML_BYTES;

#[cfg(target_os = "macos")]
pub(super) fn read_html() -> Option<String> {
    html_from_board(&objc2_app_kit::NSPasteboard::generalPasteboard())
}

#[cfg(target_os = "macos")]
pub(super) fn html_from_board(board: &objc2_app_kit::NSPasteboard) -> Option<String> {
    let html = board.dataForType(unsafe { objc2_app_kit::NSPasteboardTypeHTML })?;
    if html.length() > MAX_HTML_BYTES {
        return None;
    }
    String::from_utf8(html.to_vec()).ok()
}

#[cfg(windows)]
pub(super) fn read_html() -> Option<String> {
    use windows_sys::Win32::System::{
        DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW},
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
    };
    let name: Vec<u16> = "HTML Format\0".encode_utf16().collect();
    let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    if format == 0 || unsafe { OpenClipboard(std::ptr::null_mut()) } == 0 {
        return None;
    }
    struct Clipboard;
    impl Drop for Clipboard {
        fn drop(&mut self) {
            unsafe { CloseClipboard() };
        }
    }
    let _clipboard = Clipboard;
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return None;
    }
    let size = unsafe { GlobalSize(handle) };
    if size == 0 || size > MAX_HTML_BYTES {
        return None;
    }
    let pointer = unsafe { GlobalLock(handle) };
    if pointer.is_null() {
        return None;
    }
    // The clipboard owns the handle. Copy the validated range before unlocking it.
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size) };
    let text = std::str::from_utf8(bytes.split(|byte| *byte == 0).next().unwrap_or_default())
        .ok()
        .and_then(|s| s.find('<').map(|start| s[start..].to_string()));
    unsafe { GlobalUnlock(handle) };
    text
}

#[cfg(not(any(target_os = "macos", windows)))]
pub(super) fn read_html() -> Option<String> {
    None
}
