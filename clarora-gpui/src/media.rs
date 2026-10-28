use anyhow::{Context, Result, ensure};
use libloading::Library;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_int, c_void},
    future::Future,
    path::Path,
    pin::Pin,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    task::{Context as TaskContext, Poll, Waker},
    thread,
    time::{Duration, Instant},
};
#[repr(C)]
struct Param {
    kind: c_int,
    data: *mut c_void,
}
#[repr(C)]
union NodeData {
    string: *mut c_char,
    flag: c_int,
    integer: i64,
    double: f64,
    list: *mut NodeList,
}
#[repr(C)]
struct Node {
    data: NodeData,
    format: c_int,
}
#[repr(C)]
struct NodeList {
    num: c_int,
    values: *mut Node,
    keys: *mut *mut c_char,
}
#[repr(C)]
struct Event {
    id: c_int,
    error: c_int,
    reply: u64,
    data: *mut c_void,
}
#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}
type Handle = *mut c_void;
struct Api {
    _library: Library,
    create: unsafe extern "C" fn() -> Handle,
    initialize: unsafe extern "C" fn(Handle) -> c_int,
    destroy: unsafe extern "C" fn(Handle),
    option: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    property: unsafe extern "C" fn(Handle, *const c_char, c_int, *mut c_void) -> c_int,
    set: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    command: unsafe extern "C" fn(Handle, *const *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(Handle, f64) -> *const Event,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    node_free: unsafe extern "C" fn(*mut Node),
    render_create: unsafe extern "C" fn(*mut Handle, Handle, *mut Param) -> c_int,
    render_free: unsafe extern "C" fn(Handle),
    render: unsafe extern "C" fn(Handle, *mut Param) -> c_int,
    update: unsafe extern "C" fn(Handle) -> u64,
    update_callback:
        unsafe extern "C" fn(Handle, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
    wakeup_callback:
        unsafe extern "C" fn(Handle, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
}
impl Api {
    unsafe fn load() -> Result<Self> {
        let names = if cfg!(windows) {
            vec!["mpv-2.dll", "libmpv-2.dll"]
        } else {
            vec!["libmpv.2.dylib", "libmpv.dylib"]
        };
        let mut loaded = None;
        for path in crate::runtime::library_candidates(&names) {
            if let Ok(lib) = unsafe { Library::new(path) } {
                loaded = Some(lib);
                break;
            }
        }
        let lib = loaded.context(
            "未找到 libmpv。Mac 安装 brew install mpv；Windows 将 mpv-2.dll 与运行依赖放在程序旁",
        )?;
        macro_rules! sym {
            ($name:literal) => {
                *unsafe { lib.get(concat!($name, "\0").as_bytes())? }
            };
        }
        Ok(Self {
            create: sym!("mpv_create"),
            initialize: sym!("mpv_initialize"),
            destroy: sym!("mpv_terminate_destroy"),
            option: sym!("mpv_set_option_string"),
            property: sym!("mpv_get_property"),
            set: sym!("mpv_set_property_string"),
            command: sym!("mpv_command"),
            wait_event: sym!("mpv_wait_event"),
            error_string: sym!("mpv_error_string"),
            node_free: sym!("mpv_free_node_contents"),
            render_create: sym!("mpv_render_context_create"),
            render_free: sym!("mpv_render_context_free"),
            render: sym!("mpv_render_context_render"),
            update: sym!("mpv_render_context_update"),
            update_callback: sym!("mpv_render_context_set_update_callback"),
            wakeup_callback: sym!("mpv_set_wakeup_callback"),
            _library: lib,
        })
    }
}
unsafe fn node_json(node: &Node) -> Value {
    unsafe {
        match node.format {
            1 => {
                if node.data.string.is_null() {
                    Value::Null
                } else {
                    json!(CStr::from_ptr(node.data.string).to_string_lossy())
                }
            }
            3 => json!(node.data.flag != 0),
            4 => json!(node.data.integer),
            5 => json!(node.data.double),
            7 | 8 => {
                let list = &*node.data.list;
                if list.num <= 0 {
                    return if node.format == 7 {
                        json!([])
                    } else {
                        json!({})
                    };
                }
                let values = std::slice::from_raw_parts(list.values, list.num as usize);
                if node.format == 7 {
                    Value::Array(values.iter().map(|n| node_json(n)).collect())
                } else {
                    let keys = std::slice::from_raw_parts(list.keys, list.num.max(0) as usize);
                    Value::Object(
                        keys.iter()
                            .zip(values)
                            .map(|(k, v)| {
                                (
                                    CStr::from_ptr(*k).to_string_lossy().into_owned(),
                                    node_json(v),
                                )
                            })
                            .collect(),
                    )
                }
            }
            _ => Value::Null,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct SentenceRepeat {
    pub request_id: u64,
    pub path: String,
    pub start: f64,
    pub end: f64,
    /// Includes the first play. None repeats until explicitly stopped.
    pub total_plays: Option<u32>,
    pub current_play: u32,
    pub finished: bool,
}
struct RunningSentenceRepeat {
    state: SentenceRepeat,
    initial_seek_seen: bool,
    armed: bool,
}
/// Keep a timeline seek inside one sentence. The open end is intentionally just
/// before B, so a paused click at/after B cannot itself finish the sentence.
pub fn sentence_seek_target(start: f64, end: f64, time: f64) -> Result<f64> {
    ensure!(
        start.is_finite() && end.is_finite() && time.is_finite(),
        "字幕定位时间无效"
    );
    ensure!(start >= 0. && end > start, "字幕定位范围无效");
    let margin = 0.001_f64.min((end - start) * 0.1);
    Ok(time.clamp(start, (end - margin).max(start)))
}
#[derive(Clone, Default, Debug, serde::Serialize)]
pub struct RenderStats {
    pub rendered_frames: u64,
    pub render_microseconds: u64,
    pub allocated_bytes: u64,
    /// Actual libmpv hwdec-current. `no` means software decoding.
    pub decoder: String,
    pub pixel_format: String,
}
#[derive(Default)]
struct FrameEventState {
    revision: AtomicU64,
    next_id: AtomicU64,
    waiters: Mutex<BTreeMap<u64, Waker>>,
}
/// Broadcast revision notification: no frame queue, and each subscriber keeps
/// its own waker. A late subscriber observes the newest revision immediately.
#[derive(Clone, Default)]
pub struct FrameEvents(Arc<FrameEventState>);
impl FrameEvents {
    pub fn revision(&self) -> u64 {
        self.0.revision.load(Ordering::Acquire)
    }
    pub fn changed(&self, after: u64) -> FrameChanged {
        FrameChanged {
            events: self.clone(),
            after,
            id: self.0.next_id.fetch_add(1, Ordering::Relaxed),
        }
    }
    fn publish(&self) -> u64 {
        let (revision, waiters) = {
            let mut waiters = self.0.waiters.lock().unwrap();
            let revision = self.0.revision.fetch_add(1, Ordering::AcqRel) + 1;
            (revision, std::mem::take(&mut *waiters))
        };
        for (_, waker) in waiters {
            waker.wake();
        }
        revision
    }
}
pub struct FrameChanged {
    events: FrameEvents,
    after: u64,
    id: u64,
}
impl Future for FrameChanged {
    type Output = u64;
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<u64> {
        let mut waiters = self.events.0.waiters.lock().unwrap();
        let revision = self.events.revision();
        if revision != self.after {
            waiters.remove(&self.id);
            Poll::Ready(revision)
        } else {
            waiters.insert(self.id, cx.waker().clone());
            Poll::Pending
        }
    }
}
impl Drop for FrameChanged {
    fn drop(&mut self) {
        self.events.0.waiters.lock().unwrap().remove(&self.id);
    }
}
#[derive(Clone, Default)]
pub struct Playback {
    pub initialized: bool,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub eof: bool,
    pub path: String,
    pub subtitle: String,
    pub subtitle_is_bitmap: bool,
    pub primary_subtitles_visible: bool,
    pub secondary_subtitles_visible: bool,
    pub tracks: Vec<Value>,
    pub frame: Option<Arc<gpui::RenderImage>>,
    pub frame_revision: u64,
    pub frame_width: u32,
    pub frame_height: u32,
    pub render_stats: RenderStats,
    pub sentence_repeat: Option<SentenceRepeat>,
    pub error: String,
}
/// Compare every state field consumed by the UI, including paused errors and
/// clearing an old frame. Progress alone cannot signal those transitions.
pub fn visible_changed(before: &Playback, after: &Playback) -> bool {
    let frame_changed = match (&before.frame, &after.frame) {
        (Some(left), Some(right)) => !Arc::ptr_eq(left, right),
        (None, None) => false,
        _ => true,
    };
    frame_changed || controls_changed(before, after)
}
/// Controls and subtitles retain their own clock; decoding a new frame alone
/// must not make the 50 ms controls poll responsible for frame delivery.
pub fn controls_changed(before: &Playback, after: &Playback) -> bool {
    before.initialized != after.initialized
        || (before.position - after.position).abs() > 0.01
        || before.duration != after.duration
        || before.paused != after.paused
        || before.eof != after.eof
        || before.path != after.path
        || before.subtitle != after.subtitle
        || before.subtitle_is_bitmap != after.subtitle_is_bitmap
        || before.primary_subtitles_visible != after.primary_subtitles_visible
        || before.secondary_subtitles_visible != after.secondary_subtitles_visible
        || before.tracks != after.tracks
        || before.sentence_repeat != after.sentence_repeat
        || before.error != after.error
}
enum Message {
    Wake,
    Command(Vec<String>),
    Set(String, String),
    Subtitles(bool),
    RepeatSentence(SentenceRepeat),
    SeekSentenceRepeat {
        path: String,
        request_id: u64,
        time: f64,
    },
    ClearSentenceRepeat,
    Close,
}
pub struct Player {
    tx: mpsc::Sender<Message>,
    next_sentence_repeat_id: AtomicU64,
    pub state: Arc<Mutex<Playback>>,
    worker: Option<thread::JoinHandle<()>>,
    render_signal: Arc<RenderSignal>,
    frame_events: FrameEvents,
}
impl Player {
    pub fn new() -> Self {
        Self::with_audio_device(true)
    }
    /// Render/clock validation on CI without requiring an interactive sound device.
    pub fn without_audio_device() -> Self {
        Self::with_audio_device(false)
    }
    fn with_audio_device(audio_device: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(Playback {
            paused: true,
            ..Default::default()
        }));
        let shared = state.clone();
        let render_signal = Arc::new(RenderSignal::default());
        let worker_signal = render_signal.clone();
        let frame_events = FrameEvents::default();
        let worker_events = frame_events.clone();
        let wake = Arc::new(CoreWake {
            tx: tx.clone(),
            pending: std::sync::atomic::AtomicBool::new(false),
        });
        let worker = thread::spawn(move || {
            if let Err(e) = worker(
                rx,
                shared.clone(),
                audio_device,
                worker_signal,
                worker_events.clone(),
                wake,
            ) {
                if let Ok(mut s) = shared.lock() {
                    s.error = e.to_string();
                }
                worker_events.publish();
            }
        });
        Self {
            tx,
            next_sentence_repeat_id: AtomicU64::new(1),
            state,
            worker: Some(worker),
            render_signal,
            frame_events,
        }
    }
    pub fn command(&self, args: impl IntoIterator<Item = impl Into<String>>) {
        let _ = self
            .tx
            .send(Message::Command(args.into_iter().map(Into::into).collect()));
    }
    pub fn frame_events(&self) -> FrameEvents {
        self.frame_events.clone()
    }
    /// Physical viewport pixels. Updates are coalesced in one shared target,
    /// so layout passes cannot create a queue of stale resize messages.
    pub fn set_video_target(&self, width: u32, height: u32, visible: bool) {
        let target = VideoTarget {
            width,
            height,
            visible: visible && width > 0 && height > 0,
        };
        let mut state = self.render_signal.state.lock().unwrap();
        if state.target != target {
            state.target = target;
            state.dirty = true;
            self.render_signal.changed.notify_one();
        }
    }
    pub fn set(&self, key: &str, value: impl ToString) {
        let _ = self.tx.send(Message::Set(key.into(), value.to_string()));
    }
    pub fn set_subtitles_visible(&self, visible: bool) {
        let _ = self.tx.send(Message::Subtitles(visible));
    }
    /// Repeats one fixed subtitle range without reloading the active media.
    pub fn repeat_sentence(
        &self,
        path: &Path,
        start: f64,
        end: f64,
        total_plays: Option<u32>,
    ) -> Result<u64> {
        ensure!(start.is_finite() && end.is_finite(), "字幕时间无效");
        ensure!(start >= 0. && end > start, "字幕结束时间必须晚于开始时间");
        ensure!(
            total_plays.is_none_or(|count| count > 0 && count <= i32::MAX as u32),
            "重复次数无效"
        );
        ensure!(!path.as_os_str().is_empty(), "请先打开视频");
        let request_id = self.next_sentence_repeat_id.fetch_add(1, Ordering::Relaxed);
        self.tx
            .send(Message::RepeatSentence(SentenceRepeat {
                request_id,
                path: path.to_string_lossy().into_owned(),
                start,
                end,
                total_plays,
                current_play: 1,
                finished: false,
            }))
            .context("播放器已关闭")?;
        Ok(request_id)
    }
    pub fn clear_sentence_repeat(&self) {
        let _ = self.tx.send(Message::ClearSentenceRepeat);
    }
    /// Seek an active finite sentence repeat without resetting its count or
    /// pause state. Stale/finished requests and infinite mode are left untouched.
    pub fn seek_sentence_repeat(&self, path: &Path, request_id: u64, time: f64) -> Result<()> {
        ensure!(!path.as_os_str().is_empty(), "请先打开视频");
        ensure!(time.is_finite(), "播放位置无效");
        self.tx
            .send(Message::SeekSentenceRepeat {
                path: path.to_string_lossy().into_owned(),
                request_id,
                time,
            })
            .context("播放器已关闭")
    }
    pub fn open(&self, path: &Path) {
        self.open_range(path, 0., 0., false);
    }
    pub fn open_range(&self, path: &Path, start: f64, end: f64, paused: bool) {
        self.clear_sentence_repeat();
        self.set("pause", "yes");
        self.set("ab-loop-a", "no");
        self.set("ab-loop-b", "no");
        self.set("loop-file", "no");
        let options = if end > start {
            format!("start={start},end={end}")
        } else {
            format!("start={start},end=none")
        };
        self.command([
            "loadfile".into(),
            path.to_string_lossy().into_owned(),
            "replace".into(),
            "-1".into(),
            options,
        ]);
        self.set("pause", if paused { "yes" } else { "no" });
    }
    pub fn snapshot(&self) -> Playback {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }
}
impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Close);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
struct ContextOwner<'a> {
    api: &'a Api,
    handle: Handle,
}
impl Drop for ContextOwner<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.api.wakeup_callback)(self.handle, None, std::ptr::null_mut());
            (self.api.destroy)(self.handle)
        }
    }
}
struct CoreWake {
    tx: mpsc::Sender<Message>,
    pending: std::sync::atomic::AtomicBool,
}
unsafe extern "C" fn wake_core(data: *mut c_void) {
    let signal = unsafe { &*(data as *const CoreWake) };
    if !signal.pending.swap(true, Ordering::AcqRel) {
        let _ = signal.tx.send(Message::Wake);
    }
}
#[derive(Clone, Copy, PartialEq)]
struct VideoTarget {
    width: u32,
    height: u32,
    visible: bool,
}
struct RenderSignalState {
    target: VideoTarget,
    source: (u32, u32),
    generation: u64,
    dirty: bool,
    pending: bool,
    closed: bool,
}
struct RenderSignal {
    state: Mutex<RenderSignalState>,
    changed: Condvar,
}
impl Default for RenderSignal {
    fn default() -> Self {
        Self {
            state: Mutex::new(RenderSignalState {
                target: VideoTarget {
                    width: 960,
                    height: 540,
                    visible: true,
                },
                source: (0, 0),
                generation: 0,
                dirty: false,
                pending: false,
                closed: false,
            }),
            changed: Condvar::new(),
        }
    }
}
impl RenderSignal {
    fn source(&self, source: (u32, u32), new_media: bool) {
        let mut state = self.state.lock().unwrap();
        if state.source != source || new_media {
            state.source = source;
            state.generation += 1;
            state.dirty = true;
            self.changed.notify_one();
        }
    }
}
unsafe extern "C" fn wake_render(data: *mut c_void) {
    let signal = unsafe { &*(data as *const RenderSignal) };
    let mut state = signal.state.lock().unwrap();
    state.pending = true;
    signal.changed.notify_one();
}
fn clear_frame(shared: &Mutex<Playback>, events: &FrameEvents) {
    let mut state = shared.lock().unwrap();
    if state.frame.take().is_some() || state.frame_width != 0 || state.frame_height != 0 {
        state.frame_width = 0;
        state.frame_height = 0;
        state.frame_revision = events.revision() + 1;
        events.publish();
    }
}
/// Select content pixels, rather than letterboxing into an arbitrary target.
/// Never upscale a source, and bound CPU conversion + each GPUI upload to 14 MiB.
fn render_dimensions(target: VideoTarget, source: (u32, u32)) -> Option<(u32, u32)> {
    if !target.visible || source.0 == 0 || source.1 == 0 {
        return None;
    }
    let (sw, sh) = (f64::from(source.0), f64::from(source.1));
    let scale = (f64::from(target.width) / sw)
        .min(f64::from(target.height) / sh)
        .min(1.)
        .min(4096. / sw.max(sh))
        .min(((2560. * 1440.) / (sw * sh)).sqrt());
    Some((
        (sw * scale).floor().max(1.) as u32,
        (sh * scale).floor().max(1.) as u32,
    ))
}
struct RenderWorker {
    signal: Arc<RenderSignal>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for RenderWorker {
    fn drop(&mut self) {
        {
            let mut state = self.signal.state.lock().unwrap();
            state.closed = true;
            self.signal.changed.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl RenderWorker {
    fn start(
        handle: Handle,
        signal: Arc<RenderSignal>,
        shared: Arc<Mutex<Playback>>,
        events: FrameEvents,
    ) -> Result<Self> {
        let handle = handle as usize;
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread_signal = signal.clone();
        let thread = thread::spawn(move || {
            // The render thread only calls mpv_render_* APIs. Core commands and
            // properties on this same thread can stall or deadlock libmpv.
            let result = (|| -> Result<()> {
                unsafe {
                    let api = Api::load()?;
                    let mut render = std::ptr::null_mut();
                    let mut create = [
                        Param {
                            kind: 1,
                            data: c"sw".as_ptr() as _,
                        },
                        Param {
                            kind: 0,
                            data: std::ptr::null_mut(),
                        },
                    ];
                    ensure!(
                        (api.render_create)(&mut render, handle as Handle, create.as_mut_ptr())
                            >= 0,
                        "libmpv 软件帧接口初始化失败"
                    );
                    struct Owner<'a> {
                        api: &'a Api,
                        render: Handle,
                    }
                    impl Drop for Owner<'_> {
                        fn drop(&mut self) {
                            unsafe {
                                (self.api.update_callback)(self.render, None, std::ptr::null_mut());
                                (self.api.render_free)(self.render);
                            }
                        }
                    }
                    let _owner = Owner { api: &api, render };
                    (api.update_callback)(
                        render,
                        Some(wake_render),
                        Arc::as_ptr(&thread_signal) as *mut _,
                    );
                    let _ = ready_tx.send(Ok(()));
                    // Prefer a native alpha-bearing packed format to avoid a
                    // Rust per-pixel alpha pass. Older mpv builds may only
                    // accept the baseline bgr0 format; keep that fallback.
                    let mut alpha_format = true;
                    loop {
                        let (target, source, generation, dirty) = {
                            let mut state = thread_signal.state.lock().unwrap();
                            while !state.pending && !state.dirty && !state.closed {
                                state = thread_signal.changed.wait(state).unwrap();
                            }
                            if state.closed {
                                break;
                            }
                            let next = (state.target, state.source, state.generation, state.dirty);
                            state.pending = false;
                            state.dirty = false;
                            next
                        };
                        let updates = (api.update)(render);
                        let Some((width, height)) = render_dimensions(target, source) else {
                            // Acknowledge hidden frames without allocating pixels.
                            // Otherwise mpv can wait for its embedding application.
                            if updates & 1 != 0 {
                                let mut skip = 1i32;
                                let mut block = 0i32;
                                let mut params = [
                                    Param {
                                        kind: 13,
                                        data: &mut skip as *mut _ as _,
                                    },
                                    Param {
                                        kind: 12,
                                        data: &mut block as *mut _ as _,
                                    },
                                    Param {
                                        kind: 0,
                                        data: std::ptr::null_mut(),
                                    },
                                ];
                                (api.render)(render, params.as_mut_ptr());
                            }
                            clear_frame(&shared, &events);
                            continue;
                        };
                        if updates & 1 == 0 && !dirty {
                            continue;
                        }
                        let started = Instant::now();
                        let mut dimensions = [width as i32, height as i32];
                        let mut stride = width as usize * 4;
                        let mut block = 0i32;
                        // Write directly into the immutable image's eventual owner.
                        // There is no scratch-frame -> image full-frame copy.
                        let mut buffer = vec![0u8; stride * height as usize];
                        ensure!((buffer.as_ptr() as usize) % 4 == 0, "视频帧像素未对齐");
                        let mut params = [
                            Param {
                                kind: 17,
                                data: dimensions.as_mut_ptr() as _,
                            },
                            Param {
                                kind: 18,
                                data: if alpha_format {
                                    c"bgra".as_ptr()
                                } else {
                                    c"bgr0".as_ptr()
                                } as _,
                            },
                            Param {
                                kind: 19,
                                data: &mut stride as *mut _ as _,
                            },
                            Param {
                                kind: 20,
                                data: buffer.as_mut_ptr() as _,
                            },
                            Param {
                                kind: 12,
                                data: &mut block as *mut _ as _,
                            },
                            Param {
                                kind: 0,
                                data: std::ptr::null_mut(),
                            },
                        ];
                        let mut status = (api.render)(render, params.as_mut_ptr());
                        if status < 0 && alpha_format {
                            alpha_format = false;
                            params[1].data = c"bgr0".as_ptr() as _;
                            status = (api.render)(render, params.as_mut_ptr());
                        }
                        ensure!(status >= 0, "视频帧渲染失败 ({status})");
                        if !alpha_format {
                            for pixel in buffer.chunks_exact_mut(4) {
                                pixel[3] = 255;
                            }
                        }
                        let image = image::RgbaImage::from_raw(width, height, buffer)
                            .context("视频帧大小")?;
                        let image =
                            Arc::new(gpui::RenderImage::new(vec![image::Frame::new(image)]));
                        let current = thread_signal.state.lock().unwrap();
                        // A late render from a stopped or replaced file must never
                        // publish on top of the new media or a hidden surface.
                        if current.generation != generation || current.target != target {
                            continue;
                        }
                        let mut state = shared.lock().unwrap();
                        state.frame = Some(image);
                        state.frame_width = width;
                        state.frame_height = height;
                        state.render_stats.rendered_frames += 1;
                        state.render_stats.pixel_format =
                            if alpha_format { "bgra" } else { "bgr0" }.into();
                        state.render_stats.render_microseconds +=
                            started.elapsed().as_micros() as u64;
                        state.render_stats.allocated_bytes +=
                            u64::from(width) * u64::from(height) * 4;
                        state.frame_revision = events.revision() + 1;
                        events.publish();
                    }
                    Ok(())
                }
            })();
            if let Err(error) = result {
                let message = error.to_string();
                let _ = ready_tx.send(Err(message.clone()));
                shared.lock().unwrap().error = message;
                clear_frame(&shared, &events);
                events.publish();
            }
        });
        let owner = Self {
            signal,
            thread: Some(thread),
        };
        ready_rx
            .recv()
            .context("视频渲染线程未启动")?
            .map_err(anyhow::Error::msg)?;
        Ok(owner)
    }
}
unsafe fn set_native(api: &Api, handle: Handle, key: &str, value: &str) -> Result<()> {
    let status = unsafe {
        (api.set)(
            handle,
            CString::new(key)?.as_ptr(),
            CString::new(value)?.as_ptr(),
        )
    };
    ensure!(status >= 0, "播放器设置失败：{key} ({status})");
    Ok(())
}
unsafe fn clear_native_sentence_repeat(api: &Api, handle: Handle) -> Result<()> {
    let mut first_error = None;
    for (key, value) in [
        ("ab-loop-a", "no"),
        ("ab-loop-b", "no"),
        ("ab-loop-count", "inf"),
        ("end", "none"),
    ] {
        if let Err(error) = unsafe { set_native(api, handle, key, value) } {
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}
unsafe fn native_path(api: &Api, handle: Handle) -> String {
    let mut node: Node = unsafe { std::mem::zeroed() };
    if unsafe { (api.property)(handle, c"path".as_ptr(), 6, &mut node as *mut _ as _) } < 0 {
        return String::new();
    }
    let path = unsafe { node_json(&node) }
        .as_str()
        .unwrap_or("")
        .to_owned();
    unsafe { (api.node_free)(&mut node) };
    path
}
unsafe fn prepare_sentence_repeat(
    api: &Api,
    handle: Handle,
    state: &mut SentenceRepeat,
) -> Result<()> {
    unsafe {
        ensure!(
            native_path(api, handle) == state.path,
            "当前视频已切换，请重新选择字幕句"
        );
        let mut duration = 0.;
        if (api.property)(
            handle,
            c"duration".as_ptr(),
            5,
            &mut duration as *mut _ as _,
        ) >= 0
            && duration > 0.
        {
            state.end = state.end.min(duration);
            ensure!(state.start < state.end, "字幕时间超出视频范围");
        }
    }
    Ok(())
}
unsafe fn start_sentence_repeat(
    api: &Api,
    handle: Handle,
    state: SentenceRepeat,
) -> Result<RunningSentenceRepeat> {
    unsafe {
        set_native(api, handle, "pause", "yes")?;
        clear_native_sentence_repeat(api, handle)?;
        // libmpv counts seek-backs, so three total plays require two loops.
        // Its native end boundary pauses the last play precisely, even when a
        // short sentence finishes between worker/UI snapshots.
        if let Some(total) = state.total_plays {
            set_native(api, handle, "end", &state.end.to_string())?;
            set_native(api, handle, "ab-loop-count", &(total - 1).to_string())?;
        }
        set_native(api, handle, "loop-file", "no")?;
        set_native(api, handle, "ab-loop-a", &state.start.to_string())?;
        set_native(api, handle, "ab-loop-b", &state.end.to_string())?;
        let position = CString::new(state.start.to_string())?;
        let command = [
            c"seek".as_ptr(),
            position.as_ptr(),
            c"absolute+exact".as_ptr(),
            std::ptr::null(),
        ];
        ensure!(
            (api.command)(handle, command.as_ptr()) >= 0,
            "无法定位字幕句"
        );
        set_native(api, handle, "pause", "no")?;
    }
    Ok(RunningSentenceRepeat {
        state,
        initial_seek_seen: false,
        armed: false,
    })
}
unsafe fn seek_running_sentence_repeat(
    api: &Api,
    handle: Handle,
    repeat: &mut RunningSentenceRepeat,
    time: f64,
) -> Result<()> {
    let Some(total) = repeat.state.total_plays else {
        return Ok(());
    };
    unsafe {
        let target = sentence_seek_target(repeat.state.start, repeat.state.end, time)?;
        let mut remaining = -1i64;
        let mut paused = 1;
        let mut eof = 0;
        (api.property)(
            handle,
            c"remaining-ab-loops".as_ptr(),
            4,
            &mut remaining as *mut _ as _,
        );
        ensure!(
            (api.property)(handle, c"pause".as_ptr(), 3, &mut paused as *mut _ as _) >= 0,
            "无法读取当前播放状态"
        );
        (api.property)(handle, c"eof-reached".as_ptr(), 3, &mut eof as *mut _ as _);
        // Completion can precede the worker's next event poll. Do not revive a
        // completed request merely because its published flag has not caught up.
        if remaining == 0 && eof != 0 && paused != 0 {
            return Ok(());
        }
        let current_play = if remaining >= 0 {
            total.saturating_sub(remaining as u32).max(1).min(total)
        } else {
            repeat.state.current_play
        };
        let position = CString::new(target.to_string())?;
        let command = [
            c"seek".as_ptr(),
            position.as_ptr(),
            c"absolute+exact".as_ptr(),
            std::ptr::null(),
        ];
        ensure!(
            (api.command)(handle, command.as_ptr()) >= 0,
            "无法定位当前字幕句"
        );
        // Native seeks may rearm AB properties. Preserve only the iterations
        // still owed by this request, rather than starting three plays again.
        set_native(
            api,
            handle,
            "ab-loop-count",
            &(total - current_play).to_string(),
        )?;
        set_native(api, handle, "pause", if paused != 0 { "yes" } else { "no" })?;
        repeat.state.current_play = current_play;
        repeat.initial_seek_seen = false;
        repeat.armed = false;
    }
    Ok(())
}
fn worker(
    rx: mpsc::Receiver<Message>,
    shared: Arc<Mutex<Playback>>,
    audio_device: bool,
    render_signal: Arc<RenderSignal>,
    frame_events: FrameEvents,
    core_wake: Arc<CoreWake>,
) -> Result<()> {
    unsafe {
        let api = Api::load()?;
        let handle = (api.create)();
        ensure!(!handle.is_null(), "无法创建播放器");
        let _owned = ContextOwner { api: &api, handle };
        for (k, v) in [
            ("vo", "libmpv"),
            ("hwdec", "auto-copy"),
            ("hwdec-software-fallback", "yes"),
            ("profile", "sw-fast"),
            ("terminal", "no"),
            ("idle", "yes"),
            ("keep-open", "yes"),
            ("pause", "yes"),
            // GPUI draws the primary text subtitle; keep decoding for sub-text.
            ("sub-visibility", "no"),
        ] {
            (api.option)(handle, CString::new(k)?.as_ptr(), CString::new(v)?.as_ptr());
        }
        if !audio_device {
            (api.option)(handle, c"ao".as_ptr(), c"null".as_ptr());
        }
        ensure!((api.initialize)(handle) >= 0, "libmpv 初始化失败");
        (api.wakeup_callback)(handle, Some(wake_core), Arc::as_ptr(&core_wake) as *mut _);
        let _renderer = RenderWorker::start(
            handle,
            render_signal.clone(),
            shared.clone(),
            frame_events.clone(),
        )?;
        shared.lock().unwrap().initialized = true;
        let mut last_sample = Instant::now() - Duration::from_secs(1);
        let mut video_ready = false;
        let mut subtitle_is_bitmap = false;
        let mut subtitles_visible = true;
        let mut sentence_repeat: Option<RunningSentenceRepeat> = None;
        loop {
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(Message::Wake) => {
                    core_wake.pending.store(false, Ordering::Release);
                }
                Ok(Message::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Message::Command(args)) => {
                    if matches!(args.first().map(String::as_str), Some("loadfile" | "stop")) {
                        video_ready = false;
                        render_signal.source((0, 0), true);
                        clear_frame(&shared, &frame_events);
                    }
                    let cancels_repeat = matches!(
                        args.first().map(String::as_str),
                        Some("loadfile" | "stop" | "seek")
                    ) || sentence_repeat
                        .as_ref()
                        .is_some_and(|repeat| repeat.state.finished)
                        && args.first().is_some_and(|arg| arg == "cycle")
                        && args.get(1).is_some_and(|arg| arg == "pause");
                    if cancels_repeat && sentence_repeat.take().is_some() {
                        clear_native_sentence_repeat(&api, handle)?;
                        shared.lock().unwrap().sentence_repeat = None;
                    }
                    let strings = args
                        .iter()
                        .map(|s| CString::new(s.as_str()))
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    let mut pointers = strings.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();
                    pointers.push(std::ptr::null());
                    let status = (api.command)(handle, pointers.as_ptr());
                    if status < 0 {
                        shared.lock().unwrap().error = format!("播放命令失败 ({status})");
                    }
                }
                Ok(Message::Set(key, value)) => {
                    if key == "pause"
                        && value == "no"
                        && sentence_repeat
                            .as_ref()
                            .is_some_and(|repeat| repeat.state.finished)
                    {
                        sentence_repeat = None;
                        shared.lock().unwrap().sentence_repeat = None;
                    }
                    let status = (api.set)(
                        handle,
                        CString::new(key)?.as_ptr(),
                        CString::new(value)?.as_ptr(),
                    );
                    if status < 0 {
                        shared.lock().unwrap().error = format!("播放器设置失败：{status}");
                    }
                }
                Ok(Message::Subtitles(visible)) => {
                    subtitles_visible = visible;
                    let main = if visible && subtitle_is_bitmap {
                        c"yes"
                    } else {
                        c"no"
                    };
                    ensure!(
                        (api.set)(handle, c"sub-visibility".as_ptr(), main.as_ptr()) >= 0,
                        "无法切换主字幕显示"
                    );
                    ensure!(
                        (api.set)(
                            handle,
                            c"secondary-sub-visibility".as_ptr(),
                            if visible {
                                c"yes".as_ptr()
                            } else {
                                c"no".as_ptr()
                            }
                        ) >= 0,
                        "无法切换第二字幕显示"
                    );
                }
                Ok(Message::RepeatSentence(mut state)) => {
                    if let Err(error) = prepare_sentence_repeat(&api, handle, &mut state) {
                        shared.lock().unwrap().error = error.to_string();
                    } else {
                        match start_sentence_repeat(&api, handle, state) {
                            Ok(repeat) => {
                                let mut shared = shared.lock().unwrap();
                                shared.error.clear();
                                shared.sentence_repeat = Some(repeat.state.clone());
                                sentence_repeat = Some(repeat);
                            }
                            Err(error) => {
                                // Installing native options can fail midway (for example on
                                // an incompatible libmpv). Never leave a stale end boundary.
                                let _ = clear_native_sentence_repeat(&api, handle);
                                sentence_repeat = None;
                                let mut shared = shared.lock().unwrap();
                                shared.sentence_repeat = None;
                                shared.error = error.to_string();
                            }
                        }
                    }
                }
                Ok(Message::ClearSentenceRepeat) => {
                    if sentence_repeat.take().is_some() {
                        clear_native_sentence_repeat(&api, handle)?;
                        shared.lock().unwrap().sentence_repeat = None;
                    }
                }
                Ok(Message::SeekSentenceRepeat {
                    path,
                    request_id,
                    time,
                }) => {
                    if let Some(repeat) = sentence_repeat.as_mut().filter(|repeat| {
                        !repeat.state.finished
                            && repeat.state.request_id == request_id
                            && repeat.state.path == path
                            && native_path(&api, handle) == path
                    }) {
                        if let Err(error) = seek_running_sentence_repeat(&api, handle, repeat, time)
                        {
                            shared.lock().unwrap().error = error.to_string();
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            for _ in 0..32 {
                let event = (api.wait_event)(handle, 0.);
                if event.is_null() || (*event).id == 0 {
                    break;
                }
                if (*event).id == 8 {
                    shared.lock().unwrap().error.clear();
                    video_ready = true;
                    last_sample = Instant::now() - Duration::from_secs(1);
                } else if (*event).id == 7 && !(*event).data.is_null() {
                    let end = &*((*event).data as *const EndFile);
                    if end.reason == 4 {
                        video_ready = false;
                        render_signal.source((0, 0), true);
                        if sentence_repeat.take().is_some() {
                            clear_native_sentence_repeat(&api, handle)?;
                        }
                        let message =
                            CStr::from_ptr((api.error_string)(end.error)).to_string_lossy();
                        let mut state = shared.lock().unwrap();
                        state.error = format!("无法播放媒体：{message}");
                        state.frame = None;
                        state.frame_width = 0;
                        state.frame_height = 0;
                        state.frame_revision = frame_events.revision() + 1;
                        frame_events.publish();
                        state.paused = true;
                        state.sentence_repeat = None;
                    }
                }
                if let Some(repeat) = sentence_repeat.as_mut().filter(|r| !r.state.finished) {
                    if (*event).id == 20 {
                        if repeat.initial_seek_seen {
                            if repeat.state.total_plays.is_none() {
                                repeat.state.current_play =
                                    repeat.state.current_play.saturating_add(1);
                            }
                        } else {
                            repeat.initial_seek_seen = true;
                        }
                    } else if (*event).id == 21 && repeat.initial_seek_seen {
                        repeat.armed = true;
                    }
                }
            }
            if let Some(repeat) = sentence_repeat.as_mut().filter(|r| !r.state.finished) {
                let mut remaining = -1i64;
                let mut paused = 0;
                let mut eof = 0;
                (api.property)(
                    handle,
                    c"remaining-ab-loops".as_ptr(),
                    4,
                    &mut remaining as *mut _ as _,
                );
                (api.property)(handle, c"pause".as_ptr(), 3, &mut paused as *mut _ as _);
                (api.property)(handle, c"eof-reached".as_ptr(), 3, &mut eof as *mut _ as _);
                if let Some(total) = repeat.state.total_plays {
                    if remaining >= 0 {
                        repeat.state.current_play =
                            total.saturating_sub(remaining as u32).max(1).min(total);
                    }
                    if repeat.armed && remaining == 0 && eof != 0 && paused != 0 {
                        clear_native_sentence_repeat(&api, handle)?;
                        repeat.state.finished = true;
                        repeat.state.current_play = total;
                    }
                }
            }
            {
                let finished = sentence_repeat
                    .as_ref()
                    .is_some_and(|repeat| repeat.state.finished);
                let mut position = 0.;
                let has_position = finished
                    && (api.property)(
                        handle,
                        c"time-pos".as_ptr(),
                        5,
                        &mut position as *mut _ as _,
                    ) >= 0;
                let mut state = shared.lock().unwrap();
                state.sentence_repeat = sentence_repeat.as_ref().map(|repeat| repeat.state.clone());
                if finished {
                    if has_position {
                        state.position = position;
                    }
                    state.paused = true;
                    state.eof = false;
                }
            }
            if last_sample.elapsed() >= Duration::from_millis(100) {
                last_sample = Instant::now();
                let mut pos = 0.;
                let mut duration = 0.;
                let mut paused = 1;
                let mut eof = 0;
                (api.property)(handle, c"time-pos".as_ptr(), 5, &mut pos as *mut _ as _);
                (api.property)(
                    handle,
                    c"duration".as_ptr(),
                    5,
                    &mut duration as *mut _ as _,
                );
                (api.property)(handle, c"pause".as_ptr(), 3, &mut paused as *mut _ as _);
                (api.property)(handle, c"eof-reached".as_ptr(), 3, &mut eof as *mut _ as _);
                let mut node: Node = std::mem::zeroed();
                let mut tracks = vec![];
                if (api.property)(handle, c"track-list".as_ptr(), 6, &mut node as *mut _ as _) >= 0
                {
                    tracks = node_json(&node).as_array().cloned().unwrap_or_default();
                    (api.node_free)(&mut node);
                }
                let mut sid = 0i64;
                (api.property)(handle, c"sid".as_ptr(), 4, &mut sid as *mut _ as _);
                let bitmap = tracks.iter().any(|track| {
                    track["type"] == "sub"
                        && track["id"].as_i64() == Some(sid)
                        && matches!(
                            track["codec"].as_str(),
                            Some("dvd_subtitle" | "dvb_subtitle" | "hdmv_pgs_subtitle" | "xsub")
                        )
                });
                if bitmap != subtitle_is_bitmap {
                    // Bitmap subtitles have no sub-text; mpv must draw those.
                    let status = (api.set)(
                        handle,
                        c"sub-visibility".as_ptr(),
                        if bitmap && subtitles_visible {
                            c"yes".as_ptr()
                        } else {
                            c"no".as_ptr()
                        },
                    );
                    ensure!(status >= 0, "无法设置字幕显示方式 ({status})");
                    subtitle_is_bitmap = bitmap;
                }
                let mut media_path = String::new();
                if (api.property)(handle, c"path".as_ptr(), 6, &mut node as *mut _ as _) >= 0 {
                    media_path = node_json(&node).as_str().unwrap_or("").into();
                    (api.node_free)(&mut node);
                }
                if sentence_repeat
                    .as_ref()
                    .is_some_and(|repeat| repeat.state.path != media_path)
                {
                    clear_native_sentence_repeat(&api, handle)?;
                    sentence_repeat = None;
                }
                let mut subtitle = String::new();
                if (api.property)(handle, c"sub-text".as_ptr(), 6, &mut node as *mut _ as _) >= 0 {
                    subtitle = node_json(&node).as_str().unwrap_or("").into();
                    (api.node_free)(&mut node);
                }
                let natural_eof = eof != 0 && sentence_repeat.is_none();
                let mut width = 0i64;
                let mut height = 0i64;
                if video_ready && !natural_eof {
                    (api.property)(handle, c"dwidth".as_ptr(), 4, &mut width as *mut _ as _);
                    (api.property)(handle, c"dheight".as_ptr(), 4, &mut height as *mut _ as _);
                }
                render_signal.source((width.max(0) as u32, height.max(0) as u32), false);
                if natural_eof || media_path.is_empty() {
                    clear_frame(&shared, &frame_events);
                }
                let mut decoder = Node {
                    data: NodeData { integer: 0 },
                    format: 0,
                };
                let decoder = if (api.property)(
                    handle,
                    c"hwdec-current".as_ptr(),
                    6,
                    &mut decoder as *mut _ as _,
                ) >= 0
                {
                    let value = node_json(&decoder).as_str().unwrap_or("no").to_owned();
                    (api.node_free)(&mut decoder);
                    value
                } else {
                    "no".into()
                };
                let mut main_visible = 0;
                let mut second_visible = 0;
                (api.property)(
                    handle,
                    c"sub-visibility".as_ptr(),
                    3,
                    &mut main_visible as *mut _ as _,
                );
                (api.property)(
                    handle,
                    c"secondary-sub-visibility".as_ptr(),
                    3,
                    &mut second_visible as *mut _ as _,
                );
                // Never call libmpv with Playback locked: the VO callback and
                // renderer may need this mutex while core property calls wait.
                let mut s = shared.lock().unwrap();
                s.render_stats.decoder = decoder;
                s.position = pos;
                s.duration = duration;
                s.paused = paused != 0;
                s.eof = eof != 0
                    && !sentence_repeat
                        .as_ref()
                        .is_some_and(|repeat| repeat.state.finished);
                s.path = media_path;
                s.tracks = tracks;
                s.subtitle = subtitle;
                s.subtitle_is_bitmap = subtitle_is_bitmap;
                s.sentence_repeat = sentence_repeat.as_ref().map(|repeat| repeat.state.clone());
                s.primary_subtitles_visible = main_visible != 0;
                s.secondary_subtitles_visible = second_visible != 0;
                if s.path.is_empty() {
                    s.subtitle.clear();
                    s.tracks.clear();
                    s.position = 0.;
                    s.duration = 0.;
                    s.paused = true;
                    s.eof = false;
                }
            }
        }
        Ok(())
    }
}
