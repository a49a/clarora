use anyhow::{Context, Result, ensure};
use libloading::Library;
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
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
}
impl Api {
    unsafe fn load() -> Result<Self> {
        let candidates = if cfg!(windows) {
            vec!["mpv-2.dll", "libmpv-2.dll"]
        } else {
            vec![
                "/opt/homebrew/lib/libmpv.2.dylib",
                "/usr/local/lib/libmpv.2.dylib",
                "libmpv.2.dylib",
            ]
        };
        let mut loaded = None;
        for name in candidates {
            let path = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|x| x.join(name)));
            if let Some(p) = path {
                if let Ok(lib) = unsafe { Library::new(p) } {
                    loaded = Some(lib);
                    break;
                }
            }
            if let Ok(lib) = unsafe { Library::new(name) } {
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
#[derive(Clone, Default)]
pub struct Playback {
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub eof: bool,
    pub path: String,
    pub subtitle: String,
    pub subtitle_is_bitmap: bool,
    pub tracks: Vec<Value>,
    pub frame: Option<Arc<gpui::RenderImage>>,
    pub error: String,
}
enum Message {
    Command(Vec<String>),
    Set(String, String),
    Close,
}
pub struct Player {
    tx: mpsc::Sender<Message>,
    pub state: Arc<Mutex<Playback>>,
    worker: Option<thread::JoinHandle<()>>,
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
        let worker = thread::spawn(move || {
            if let Err(e) = worker(rx, shared.clone(), audio_device) {
                if let Ok(mut s) = shared.lock() {
                    s.error = e.to_string();
                }
            }
        });
        Self {
            tx,
            state,
            worker: Some(worker),
        }
    }
    pub fn command(&self, args: impl IntoIterator<Item = impl Into<String>>) {
        let _ = self
            .tx
            .send(Message::Command(args.into_iter().map(Into::into).collect()));
    }
    pub fn set(&self, key: &str, value: impl ToString) {
        let _ = self.tx.send(Message::Set(key.into(), value.to_string()));
    }
    pub fn open(&self, path: &Path) {
        self.open_range(path, 0., 0., false);
    }
    pub fn open_range(&self, path: &Path, start: f64, end: f64, paused: bool) {
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
    render: Handle,
}
impl Drop for ContextOwner<'_> {
    fn drop(&mut self) {
        unsafe {
            if !self.render.is_null() {
                (self.api.render_free)(self.render)
            }
            (self.api.destroy)(self.handle)
        }
    }
}
unsafe extern "C" fn wake(_: *mut c_void) {}
fn worker(
    rx: mpsc::Receiver<Message>,
    shared: Arc<Mutex<Playback>>,
    audio_device: bool,
) -> Result<()> {
    unsafe {
        let api = Api::load()?;
        let handle = (api.create)();
        ensure!(!handle.is_null(), "无法创建播放器");
        let mut owned = ContextOwner {
            api: &api,
            handle,
            render: std::ptr::null_mut(),
        };
        for (k, v) in [
            ("vo", "libmpv"),
            ("hwdec", "no"),
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
            (api.render_create)(&mut owned.render, handle, create.as_mut_ptr()) >= 0,
            "libmpv 软件帧接口初始化失败"
        );
        (api.update_callback)(owned.render, Some(wake), std::ptr::null_mut());
        let mut pixels = vec![0u64; 960 * 540 / 2];
        let mut tick = 0;
        let mut subtitle_is_bitmap = false;
        loop {
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(Message::Close) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Message::Command(args)) => {
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
                    let status = (api.set)(
                        handle,
                        CString::new(key)?.as_ptr(),
                        CString::new(value)?.as_ptr(),
                    );
                    if status < 0 {
                        shared.lock().unwrap().error = format!("播放器设置失败：{status}");
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
                } else if (*event).id == 7 && !(*event).data.is_null() {
                    let end = &*((*event).data as *const EndFile);
                    if end.reason == 4 {
                        let message =
                            CStr::from_ptr((api.error_string)(end.error)).to_string_lossy();
                        let mut state = shared.lock().unwrap();
                        state.error = format!("无法播放媒体：{message}");
                        state.frame = None;
                        state.paused = true;
                    }
                }
            }
            if (api.update)(owned.render) & 1 != 0 {
                let mut dimensions = [960i32, 540i32];
                let mut stride = 960usize * 4;
                let mut block = 0i32;
                let mut params = [
                    Param {
                        kind: 17,
                        data: dimensions.as_mut_ptr() as _,
                    },
                    Param {
                        kind: 18,
                        data: c"bgr0".as_ptr() as _,
                    },
                    Param {
                        kind: 19,
                        data: &mut stride as *mut _ as _,
                    },
                    Param {
                        kind: 20,
                        data: pixels.as_mut_ptr() as _,
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
                if (api.render)(owned.render, params.as_mut_ptr()) >= 0 {
                    let bytes =
                        std::slice::from_raw_parts(pixels.as_ptr() as *const u8, 960 * 540 * 4);
                    let mut buffer = bytes.to_vec();
                    for px in buffer.chunks_exact_mut(4) {
                        px[3] = 255;
                    }
                    let image =
                        image::RgbaImage::from_raw(960, 540, buffer).context("视频帧大小")?;
                    shared.lock().unwrap().frame =
                        Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                            image,
                        )])));
                }
            }
            tick += 1;
            if tick % 4 == 0 {
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
                        if bitmap {
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
                let mut subtitle = String::new();
                if (api.property)(handle, c"sub-text".as_ptr(), 6, &mut node as *mut _ as _) >= 0 {
                    subtitle = node_json(&node).as_str().unwrap_or("").into();
                    (api.node_free)(&mut node);
                }
                let mut s = shared.lock().unwrap();
                s.position = pos;
                s.duration = duration;
                s.paused = paused != 0;
                s.eof = eof != 0;
                s.path = media_path;
                s.tracks = tracks;
                s.subtitle = subtitle;
                s.subtitle_is_bitmap = subtitle_is_bitmap;
            }
        }
        Ok(())
    }
}
