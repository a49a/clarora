use anyhow::{Result, ensure};
use clarora_gpui::{
    data::Library,
    model_download::{self, FileSpec, Progress},
    process,
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};
fn server(
    replies: Vec<(&'static str, bool, usize, u64)>,
) -> Result<(String, thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let host = format!("http://{}", listener.local_addr()?);
    let handle = thread::spawn(move || {
        for (status, length, blocks, delay) in replies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let size = blocks * 16384;
            let header = format!(
                "HTTP/1.1 {status}\r\nConnection: close\r\n{}\r\n",
                if length {
                    format!(
                        "Content-Length: {}\r\n",
                        if status == "200 Truncated" {
                            size + 1
                        } else {
                            size
                        }
                    )
                } else {
                    String::new()
                }
            );
            if stream.write_all(header.as_bytes()).is_err() {
                continue;
            }
            for _ in 0..blocks {
                if delay > 0 {
                    thread::sleep(Duration::from_millis(delay));
                }
                if stream.write_all(&[42u8; 16384]).is_err() {
                    break;
                }
            }
        }
    });
    Ok((host, handle))
}
pub fn run(lib: &Library, results: &mut Vec<Value>) -> Result<()> {
    let directory = lib.root.join("progress-models");
    let cancel = process::cancel_token();
    let mut events: Vec<Progress> = vec![];
    let files = vec![
        FileSpec {
            name: "model.bin".into(),
            remote: "model.bin".into(),
            min_bytes: 1024,
        },
        FileSpec {
            name: "tokens.txt".into(),
            remote: "tokens.txt".into(),
            min_bytes: 1024,
        },
    ];
    let (bad, bad_worker) = server(vec![("503 Unavailable", true, 0, 0)])?;
    let (good, good_worker) = server(vec![("200 OK", true, 32, 8)])?;
    model_download::download(
        "fixture",
        &directory,
        &files[..1],
        &[&bad, &good],
        &cancel,
        &mut |p| events.push(p),
    )?;
    bad_worker.join().unwrap();
    good_worker.join().unwrap();
    ensure!(
        events.iter().any(|p| p.phase == "换源重试")
            && events
                .iter()
                .any(|p| p.downloaded > 0 && p.total.is_some() && p.phase == "下载中"),
        "已知长度进度或重试缺失"
    );
    let (unknown, worker) = server(vec![("200 OK", false, 4, 8), ("200 OK", true, 1, 0)])?;
    model_download::download(
        "multi",
        &directory,
        &files,
        &[&unknown],
        &cancel,
        &mut |p| events.push(p),
    )?;
    worker.join().unwrap();
    ensure!(
        events.iter().any(|p| p.total.is_none() && p.downloaded > 0)
            && events
                .iter()
                .any(|p| p.file_index == 2 && p.file_count == 2),
        "未知总量或多文件进度缺失"
    );
    let old = std::fs::read(directory.join("model.bin"))?;
    let (slow, worker) = server(vec![("200 OK", true, 50, 20)])?;
    let token = process::cancel_token();
    let stop = token.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(130));
        stop.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    ensure!(
        model_download::download(
            "cancel",
            &directory,
            &files[..1],
            &[&slow],
            &token,
            &mut |p| events.push(p)
        )
        .is_err(),
        "取消未停止"
    );
    ensure!(started.elapsed() < Duration::from_secs(1), "取消响应过慢");
    canceller.join().unwrap();
    worker.join().unwrap();
    ensure!(
        std::fs::read(directory.join("model.bin"))? == old
            && !directory.join("model.part").exists(),
        "取消覆盖旧文件或遗留临时文件"
    );
    let (empty, worker) = server(vec![("200 OK", true, 0, 0)])?;
    ensure!(
        model_download::download(
            "empty",
            &directory,
            &files[..1],
            &[&empty],
            &cancel,
            &mut |p| events.push(p)
        )
        .is_err(),
        "空模型误报成功"
    );
    worker.join().unwrap();
    let (truncated, worker) = server(vec![("200 Truncated", true, 2, 0)])?;
    ensure!(
        model_download::download(
            "truncated",
            &directory,
            &files[..1],
            &[&truncated],
            &cancel,
            &mut |p| events.push(p)
        )
        .is_err(),
        "截断模型误报成功"
    );
    worker.join().unwrap();
    ensure!(
        std::fs::read(directory.join("model.bin"))? == old
            && !directory.join("model.part").exists(),
        "失败覆盖旧模型或遗留.part"
    );
    ensure!(
        events
            .iter()
            .filter(|p| p.model == "truncated")
            .all(|p| p.phase != "文件完成"),
        "失败传输发出了完成事件"
    );
    ensure!(
        events
            .iter()
            .filter(|p| p.phase == "连接中")
            .all(|p| p.downloaded == 0 && p.total.is_none()),
        "新源未重置进度"
    );
    std::fs::write(
        lib.root.join("model-progress-events.json"),
        serde_json::to_vec_pretty(&events)?,
    )?;
    results.push(json!({"workflow":"模型HTTP流式进度 → 换源重置/未知总量/双文件 → 取消清理且保留旧文件/空模型拒绝","passed":true,"events":events.len()}));
    Ok(())
}
