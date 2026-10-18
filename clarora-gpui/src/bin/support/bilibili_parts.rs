use anyhow::{Result, ensure};
use clarora_gpui::{bilibili, data::Library, process};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
};

pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let endpoint = format!("http://{}/parts", listener.local_addr()?);
    let worker = thread::spawn(move || {
        let mut requests = vec![];
        for body in [
            json!({"code":0,"data":[{"page":1,"part":"第一课","duration":2},{"page":2,"part":"第二课","duration":3}]}),
            json!({"code":0,"data":[{"page":1,"part":"单集","duration":2}]}),
            json!({"code":-404,"message":"视频不存在"}),
            json!({"code":0,"data":[]}),
            json!({"code":0,"data":[{"page":1,"part":"重复"},{"page":1,"part":"重复"}]}),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).unwrap();
            requests.push(String::from_utf8_lossy(&bytes[..count]).to_string());
            let body = body.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
        requests
    });
    let cancel = process::cancel_token();
    let mut series = bilibili::fetch_from("BV1xx411c7mD?p=2", &endpoint, &cancel);
    // Bare IDs with query strings are intentionally rejected: use a full link.
    ensure!(series.is_err(), "非完整链接未拒绝");
    series = bilibili::fetch_from(
        "https://www.bilibili.com/video/BV1xx411c7mD?p=2",
        &endpoint,
        &cancel,
    );
    let mut series = series?;
    ensure!(
        series.parts.len() == 2 && series.parts[1].title == "第二课" && series.requested == 1,
        "分P列表/选中索引错误"
    );
    ensure!(
        series.url(1)? == "https://www.bilibili.com/video/BV1xx411c7mD?p=2",
        "下一P地址错误"
    );
    let path = lib.root.join("downloads/parts-e2e.mp4");
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, b"fixture")?;
    series.parts[0].path = Some(path.clone());
    series.current = Some(0);
    ensure!(series.next_for(&path) == Some(1), "当前视频下一P错误");
    ensure!(
        series.next_for(&lib.root.join("other.mp4")).is_none(),
        "本地视频串入旧队列"
    );
    series.save(lib)?;
    let restored = bilibili::Series::load(lib)?.unwrap();
    ensure!(
        restored.cached(0) == Some(path.clone()) && restored.current == Some(0),
        "队列重启/缓存恢复失败"
    );
    series.current = Some(1);
    series.parts[1].path = Some(path.clone());
    ensure!(series.next_for(&path).is_none(), "末集错误续播");
    std::fs::remove_file(&path)?;
    ensure!(restored.cached(0).is_none(), "删除的缓存仍复用");
    ensure!(
        bilibili::fetch_from("av123", &endpoint, &cancel)?
            .parts
            .len()
            == 1,
        "单P失败"
    );
    for _ in 0..3 {
        ensure!(
            bilibili::fetch_from("av123", &endpoint, &cancel).is_err(),
            "异常API未拒绝"
        );
    }
    let requests = worker.join().unwrap();
    ensure!(
        requests[0].contains("bvid=BV1xx411c7mD") && requests[1].contains("aid=123"),
        "BV/av查询错误"
    );
    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    ensure!(
        bilibili::fetch_from("av123", &endpoint, &cancel).is_err(),
        "取消后继续请求"
    );
    std::fs::write(
        lib.root.join("bilibili-parts-evidence.json"),
        serde_json::to_vec_pretty(&json!({"series":series,"requests":requests}))?,
    )?;
    let delayed = TcpListener::bind("127.0.0.1:0")?;
    let slow_url = format!("http://{}/slow", delayed.local_addr()?);
    let slow_worker = thread::spawn(move || {
        let (mut stream, _) = delayed.accept().unwrap();
        let mut data = [0; 2048];
        let _ = stream.read(&mut data);
        thread::sleep(std::time::Duration::from_millis(300));
    });
    let token = process::cancel_token();
    let stop = token.clone();
    let cancel_worker = thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(100));
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let began = std::time::Instant::now();
    ensure!(
        bilibili::fetch_from("av123", &slow_url, &token).is_err(),
        "请求等待期间取消失效"
    );
    ensure!(
        began.elapsed() < std::time::Duration::from_millis(280),
        "分P请求取消过慢"
    );
    cancel_worker.join().unwrap();
    slow_worker.join().unwrap();
    if let Ok(input) = std::env::var("CLARORA_BILI_LIVE_VIDEO") {
        let live = bilibili::fetch(&input, &process::cancel_token())?;
        std::fs::write(
            lib.root.join("bilibili-live-parts.json"),
            serde_json::to_vec_pretty(&live)?,
        )?;
        results.push(json!({"workflow":"真实bilibili分P元数据读取（不下载视频）","passed":true,"parts":live.parts.len()}));
    }
    results.push(json!({"workflow":"分P本机HTTP识别 → 指定集/缓存/SQLite重启 → 下一P/末集/本地材料隔离 → 异常与取消","passed":true}));
    Ok(())
}
