//! Isolated S3 protocol fixture: conditional writes, HEAD, listing and corruption.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Object {
    pub bytes: Vec<u8>,
    pub modified: chrono::DateTime<chrono::Utc>,
}
pub struct Cloud {
    pub address: SocketAddr,
    pub objects: Arc<Mutex<BTreeMap<String, Object>>>,
    pub requests: Arc<Mutex<Vec<Value>>>,
    pub fail_blob_at: Arc<AtomicUsize>,
    pub pause_deletes: Arc<AtomicBool>,
    pub delete_entered: Arc<AtomicBool>,
    pub pause_next_sync: Arc<AtomicBool>,
    pub sync_entered: Arc<AtomicBool>,
    pub release_sync: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn reply(socket: &mut TcpStream, status: &str, body: &[u8], extra: &str, head: bool) -> Result<()> {
    write!(
        socket,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    )?;
    if !head {
        socket.write_all(body)?;
    }
    socket.flush()?;
    Ok(())
}
impl Cloud {
    pub fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let objects = Arc::new(Mutex::new(BTreeMap::<String, Object>::new()));
        let requests = Arc::new(Mutex::new(vec![]));
        let fail_blob_at = Arc::new(AtomicUsize::new(0));
        let pause_deletes = Arc::new(AtomicBool::new(false));
        let delete_entered = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let pause_next_sync = Arc::new(AtomicBool::new(false));
        let sync_entered = Arc::new(AtomicBool::new(false));
        let release_sync = Arc::new(AtomicBool::new(false));
        let (sync_pause, sync_seen, sync_release) = (
            pause_next_sync.clone(),
            sync_entered.clone(),
            release_sync.clone(),
        );
        let (store, seen, fail, pause, entered, done) = (
            objects.clone(),
            requests.clone(),
            fail_blob_at.clone(),
            pause_deletes.clone(),
            delete_entered.clone(),
            stop.clone(),
        );
        let worker = thread::spawn(move || {
            let mut children = vec![];
            while !done.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(3));
                    continue;
                };
                let (store, seen, fail, pause, entered, done) = (
                    store.clone(),
                    seen.clone(),
                    fail.clone(),
                    pause.clone(),
                    entered.clone(),
                    done.clone(),
                );
                let (sync_pause, sync_seen, sync_release) =
                    (sync_pause.clone(), sync_seen.clone(), sync_release.clone());
                children.push(thread::spawn(move || {
                    let result=(|| -> Result<()> {
                        socket.set_nonblocking(false)?;
                        socket.set_read_timeout(Some(Duration::from_secs(5)))?;
                        socket.set_write_timeout(Some(Duration::from_secs(5)))?;
                        let mut bytes=vec![];let mut chunk=[0u8;65536];
                        let split=loop {
                            let count=socket.read(&mut chunk)?;ensure!(count>0,"header ended");bytes.extend_from_slice(&chunk[..count]);
                            if let Some(index)=bytes.windows(4).position(|p|p==b"\r\n\r\n"){break index+4;}
                            ensure!(bytes.len()<65536,"header too large");
                        };
                        let header=std::str::from_utf8(&bytes[..split])?;
                        let mut first=header.lines().next().context("request line")?.split_whitespace();
                        let method=first.next().context("method")?.to_string();let target=first.next().context("target")?;
                        let url=url::Url::parse(&format!("http://fixture{target}"))?;
                        let key=url.path().strip_prefix("/fixture-bucket/").context("bucket")?.to_string();
                        let headers=header.lines().skip(1).filter_map(|s|s.split_once(':')).map(|(k,v)|(k.to_ascii_lowercase(),v.trim().to_string())).collect::<BTreeMap<_,_>>();
                        ensure!(headers.get("authorization").is_some_and(|s|s.contains("AWS4-HMAC-SHA256")),"missing signature");
                        let length=headers.get("content-length").and_then(|s|s.parse::<usize>().ok()).unwrap_or(0);
                        ensure!(length<=64*1024*1024,"fixture payload too large");
                        let mut body=bytes[split..].to_vec();
                        while body.len()<length {let n=socket.read(&mut chunk)?;ensure!(n>0,"body ended");body.extend_from_slice(&chunk[..n]);}
                        seen.lock().unwrap().push(json!({"method":method,"key":key,"bytes":body.len(),"if_match":headers.get("if-match"),"if_none_match":headers.get("if-none-match")}));
                        if method=="DELETE" && key.contains("/objects/") {
                            entered.store(true,Ordering::SeqCst);
                            let began=Instant::now();
                            while pause.load(Ordering::SeqCst)&&!done.load(Ordering::SeqCst)&&began.elapsed()<Duration::from_secs(4){thread::sleep(Duration::from_millis(3));}
                        }
                        if method == "PUT" && key.contains("/sync/") && sync_pause.swap(false, Ordering::SeqCst) {
                            sync_seen.store(true, Ordering::SeqCst);
                            let began = Instant::now();
                            while !sync_release.load(Ordering::SeqCst) && !done.load(Ordering::SeqCst) && began.elapsed() < Duration::from_secs(8) { thread::sleep(Duration::from_millis(3)); }
                        }
                        let mut objects=store.lock().unwrap();
                        let old=objects.get(&key);let etag=old.map(|o|format!("\"{}\"",hash(&o.bytes)));
                        if headers.get("if-none-match").is_some_and(|s|s=="*")&&old.is_some()
                            ||headers.get("if-match").is_some_and(|s|Some(s)!=etag.as_ref()) {
                            return reply(&mut socket,"412 Precondition Failed",b"", "",false);
                        }
                        if method=="PUT" {
                            if key.contains("/objects/") {
                                let n=fail.load(Ordering::SeqCst);
                                if n>0&&fail.fetch_sub(1,Ordering::SeqCst)==1 {return reply(&mut socket,"503 Service Unavailable",b"<Error><Code>SlowDown</Code></Error>","",false);}
                            }
                            let etag=format!("ETag: \"{}\"\r\n",hash(&body));
                            objects.insert(key,Object{bytes:body,modified:chrono::Utc::now()});
                            reply(&mut socket,"200 OK",b"",&etag,false)
                        } else if method=="DELETE" {
                            objects.remove(&key);reply(&mut socket,"204 No Content",b"","",false)
                        } else if url.query_pairs().any(|(k,_)|k=="list-type") {
                            let prefix=url.query_pairs().find(|(k,_)|k=="prefix").map(|(_,v)|v.into_owned()).unwrap_or_default();
                            let body=format!("<ListBucketResult>{}<IsTruncated>false</IsTruncated></ListBucketResult>",objects.iter().filter(|(k,_)|k.starts_with(&prefix)).map(|(k,o)|format!("<Contents><Key>{k}</Key><Size>{}</Size><LastModified>{}</LastModified></Contents>",o.bytes.len(),o.modified.to_rfc3339())).collect::<String>());
                            reply(&mut socket,"200 OK",body.as_bytes(),"",false)
                        } else if let Some(object)=objects.get(&key) {
                            let extra=format!("ETag: \"{}\"\r\nLast-Modified: {}\r\nx-amz-meta-sha256: {}\r\n",hash(&object.bytes),object.modified.format("%a, %d %b %Y %H:%M:%S GMT"),hash(&object.bytes));
                            reply(&mut socket,"200 OK",&object.bytes,&extra,method=="HEAD")
                        } else {reply(&mut socket,"404 Not Found",b"<Error><Code>NoSuchKey</Code></Error>","",method=="HEAD")}
                    })();
                    if let Err(error)=result { eprintln!("cloud fixture connection error: {error:#}"); seen.lock().unwrap().push(json!({"connection_error":error.to_string()})); }
                }));
            }
            for child in children {
                let _ = child.join();
            }
        });
        Ok(Self {
            address,
            objects,
            requests,
            fail_blob_at,
            pause_deletes,
            delete_entered,
            pause_next_sync,
            sync_entered,
            release_sync,
            stop,
            worker: Some(worker),
        })
    }
    pub fn storage(&self) -> Result<clarora_gpui::backup::Storage> {
        clarora_gpui::backup::Storage::loopback_fixture(self.address)
    }
    pub fn blob_puts(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| {
                r["method"] == "PUT" && r["key"].as_str().is_some_and(|s| s.contains("/objects/"))
            })
            .count()
    }
    pub fn age(&self, days: i64) {
        for object in self.objects.lock().unwrap().values_mut() {
            object.modified -= chrono::Duration::days(days);
        }
    }
    pub fn evidence(&self) -> Value {
        json!({"requests":self.requests.lock().unwrap().clone(),"objects":self.objects.lock().unwrap().iter().map(|(key,o)|json!({"key":key,"bytes":o.bytes.len(),"sha256":hash(&o.bytes),"modified":o.modified})).collect::<Vec<_>>()})
    }
}
impl Drop for Cloud {
    fn drop(&mut self) {
        self.pause_deletes.store(false, Ordering::SeqCst);
        self.release_sync.store(true, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
