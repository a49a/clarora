//! Read and write legacy object-store snapshots.
use super::attachments::{StagedAttachment, attachment_stamps, check_attachments};
use super::manifest::{Manifest, collect_cancellable, map_manifest, validate};
use super::restore::finish_restore;
use crate::data::{Library, id};
use crate::object_storage::{
    Payload, Storage, UploadProgress, UploadReporter, UploadStage, cancellable, check_cancel,
};
use crate::process::Cancel;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    sync::Arc,
};

impl Storage {
    pub fn list(&self) -> Result<Vec<String>> {
        self.list_cancellable(&crate::process::cancel_token())
    }
    pub fn list_cancellable(&self, cancel: &Cancel) -> Result<Vec<String>> {
        check_cancel(cancel)?;
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let client = self.client()?;
                let prefix = format!("{}/snapshots/", self.config().prefix);
                let mut keys = vec![];
                let mut token = String::new();
                let mut seen = std::collections::HashSet::new();
                loop {
                    check_cancel(cancel)?;
                    let mut query = BTreeMap::from([
                        ("list-type".into(), "2".into()),
                        ("prefix".into(), prefix.clone()),
                    ]);
                    if !token.is_empty() {
                        query.insert("continuation-token".into(), token);
                    }
                    let xml = String::from_utf8(
                        self.request(&client, "GET", "", &query, None, cancel)
                            .await?,
                    )?;
                    ensure!(
                        !xml.contains("<!DOCTYPE") && !xml.contains("<!ENTITY"),
                        "不支持的 XML"
                    );
                    #[derive(Deserialize)]
                    #[serde(rename_all = "PascalCase")]
                    struct Listing {
                        #[serde(default)]
                        contents: Vec<Content>,
                        #[serde(default)]
                        is_truncated: bool,
                        next_continuation_token: Option<String>,
                    }
                    #[derive(Deserialize)]
                    #[serde(rename_all = "PascalCase")]
                    struct Content {
                        key: String,
                    }
                    let listing: Listing = quick_xml::de::from_str(&xml)?;
                    keys.extend(
                        listing
                            .contents
                            .into_iter()
                            .map(|c| c.key)
                            .filter(|k| k.starts_with(&prefix) && k.ends_with(".json")),
                    );
                    if !listing.is_truncated {
                        break;
                    }
                    token = listing
                        .next_continuation_token
                        .context("存储分页缺少令牌")?;
                    ensure!(
                        !token.is_empty() && seen.insert(token.clone()),
                        "存储分页循环"
                    );
                }
                keys.sort();
                keys.reverse();
                Ok(keys)
            })
    }
    pub fn upload(&self, lib: &Library, cancel: &crate::process::Cancel) -> Result<String> {
        self.upload_with_progress(lib, cancel, |_| {})
    }
    pub fn upload_with_progress(
        &self,
        lib: &Library,
        cancel: &Cancel,
        progress: impl Fn(UploadProgress) + Send + Sync + 'static,
    ) -> Result<String> {
        check_cancel(cancel)?;
        let reporter = Arc::new(UploadReporter::new(progress));
        reporter.update(true, |_| {});
        let (mut manifest, files) = collect_cancellable(lib, cancel, true)?;
        let stamps = attachment_stamps(&files)?;
        let bytes_total = files.values().try_fold(0_u64, |sum, path| {
            sum.checked_add(stamps[path].0)
                .context("备份附件总量超出支持范围")
        })?;
        reporter.update(true, |progress| {
            progress.stage = UploadStage::Uploading;
            progress.bytes_total = bytes_total;
            progress.files_total = files.len();
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let client = self.client()?;
        for (reference, path) in files {
            check_cancel(cancel)?;
            let length = stamps[&path].0;
            let expected_sent = reporter.sent() + length;
            let name: String = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .chars()
                .filter(|c| !c.is_control())
                .take(80)
                .collect();
            reporter.update(true, |progress| progress.current_name = name);
            let key = format!(
                "{}/media/{}/{}.{}",
                self.config().prefix,
                manifest.id,
                reference.trim_start_matches("media:"),
                path.extension().unwrap_or_default().to_string_lossy()
            );
            runtime.block_on(self.response(
                &client,
                "PUT",
                &key,
                &BTreeMap::new(),
                Some(Payload::File {
                    path,
                    length,
                    reporter: reporter.clone(),
                }),
                cancel,
            ))?;
            ensure!(
                reporter.sent() == expected_sent,
                "存储服务在附件发送完成前返回成功，请重试"
            );
            reporter.update(true, |progress| {
                progress.files_done += 1;
                progress.current_name.clear();
            });
            manifest.files.insert(reference, key);
        }
        check_attachments(&stamps, cancel)?;
        let key = format!("{}/snapshots/{}.json", self.config().prefix, manifest.id);
        let bytes = serde_json::to_vec(&manifest)?;
        check_cancel(cancel)?;
        reporter.update(true, |progress| {
            progress.stage = UploadStage::Publishing;
            progress.current_name.clear();
        });
        check_cancel(cancel)?;
        // Publishing the manifest is the remote commit. A lost response cannot prove rollback.
        runtime
            .block_on(self.response(
                &client,
                "PUT",
                &key,
                &BTreeMap::new(),
                Some(Payload::Bytes(bytes)),
                cancel,
            ))
            .context("备份清单发布结果未确认，请刷新云端列表核实；附件可能已上传")?;
        reporter.update(true, |progress| progress.stage = UploadStage::Completed);
        Ok(key)
    }
    pub fn restore(
        &self,
        lib: &Library,
        key: &str,
        cancel: &crate::process::Cancel,
    ) -> Result<usize> {
        if key.starts_with(&format!("{}/v3/snapshots/", self.config().prefix)) {
            return self.restore_incremental(lib, key, cancel);
        }
        check_cancel(cancel)?;
        ensure!(
            key.starts_with(&format!("{}/snapshots/", self.config().prefix)),
            "备份路径无效"
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let client = self.client()?;
        let mut manifest: Manifest = serde_json::from_slice(&runtime.block_on(self.request(
            &client,
            "GET",
            key,
            &BTreeMap::new(),
            None,
            cancel,
        ))?)?;
        check_cancel(cancel)?;
        validate(&mut manifest)?;
        ensure!(
            key == format!("{}/snapshots/{}.json", self.config().prefix, manifest.id),
            "备份清单标识不一致"
        );
        let root = lib.root.join("media").join(format!("restored-{}", id()));
        fs::create_dir_all(&root)?;
        let mut paths: HashMap<String, String> = HashMap::new();
        let files = manifest.files.clone();
        let sources = manifest.sources.clone();
        let remote_prefix = format!("{}/media/{}/", self.config().prefix, manifest.id);
        let result = (|| {
            map_manifest(&mut manifest, |reference| {
                check_cancel(cancel)?;
                if let Some(url) = sources.get(reference) {
                    return Ok(crate::bilibili::source_path(lib, url)?
                        .to_string_lossy()
                        .into_owned());
                }
                let key = files.get(reference).context("备份缺少附件")?;
                if let Some(p) = paths.get(key) {
                    return Ok(p.clone());
                }
                let prefix = remote_prefix.as_str();
                ensure!(
                    key.starts_with(prefix)
                        && regex::Regex::new(r"^[0-9]+\.[A-Za-z0-9]{1,15}$")?
                            .is_match(&key[prefix.len()..]),
                    "远端附件路径无效"
                );
                let path = root.join(&key[prefix.len()..]);
                runtime.block_on(async {
                    let mut response = self
                        .response(&client, "GET", key, &BTreeMap::new(), None, cancel)
                        .await?;
                    cancellable(cancel, async {
                        let length = response.content_length();
                        let mut output = StagedAttachment::new(&path, None)?;
                        while let Some(chunk) = response.chunk().await? {
                            output.write(&chunk, cancel)?;
                        }
                        output.finish(&path, length, cancel)
                    })
                    .await
                })?;
                let path = path.to_string_lossy().to_string();
                paths.insert(key.clone(), path.clone());
                Ok(path)
            })?;
            finish_restore(lib, &mut manifest, &root, cancel)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(root);
        }
        result
    }
}
