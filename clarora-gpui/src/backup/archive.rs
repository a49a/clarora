//! Local ZIP backup format and entry mapping.
use super::attachments::{
    MAX_ATTACHMENT_BYTES, StagedAttachment, attachment_stamps, check_attachments, copy_cancellable,
};
use super::check_cancel;
use super::manifest::{Manifest, collect_cancellable, map_manifest, validate};
use super::restore::finish_restore;
use crate::data::{Library, id};
use crate::process::Cancel;
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::Path,
};

pub fn export_local(lib: &Library, path: &Path) -> Result<()> {
    export_local_cancellable(lib, path, &crate::process::cancel_token())
}
pub fn export_local_cancellable(lib: &Library, path: &Path, cancel: &Cancel) -> Result<()> {
    let (mut manifest, files) = collect_cancellable(lib, cancel, false)?;
    let stamps = attachment_stamps(&files)?;
    let temp = path.with_file_name(format!(
        ".{}-{}.part",
        path.file_name()
            .context("备份文件名无效")?
            .to_string_lossy(),
        id()
    ));
    let result = (|| {
        let mut zip = zip::ZipWriter::new(
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?,
        );
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (reference, file) in files {
            check_cancel(cancel)?;
            let name = format!(
                "media/{}.{}",
                reference.trim_start_matches("media:"),
                file.extension().and_then(|s| s.to_str()).unwrap_or("bin")
            );
            manifest.files.insert(reference, name.clone());
            zip.start_file(name, options)?;
            copy_cancellable(fs::File::open(file)?, &mut zip, cancel)?;
        }
        check_cancel(cancel)?;
        zip.start_file("manifest.json", options)?;
        zip.write_all(serde_json::to_string(&manifest)?.as_bytes())?;
        zip.finish()?.sync_all()?;
        check_attachments(&stamps, cancel)?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub fn import_local(lib: &Library, path: &Path) -> Result<usize> {
    import_local_cancellable(lib, path, &crate::process::cancel_token())
}
pub fn import_local_cancellable(lib: &Library, path: &Path, cancel: &Cancel) -> Result<usize> {
    check_cancel(cancel)?;
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
    let mut raw = String::new();
    let entry = zip.by_name("manifest.json")?;
    ensure!(entry.size() <= 64 * 1024 * 1024, "备份清单过大");
    entry.take(64 * 1024 * 1024 + 1).read_to_string(&mut raw)?;
    check_cancel(cancel)?;
    let mut manifest: Manifest = serde_json::from_str(&raw)?;
    validate(&mut manifest)?;
    let stage = lib.root.join("media").join(format!("restored-{}", id()));
    fs::create_dir_all(&stage)?;
    let result = (|| {
        let mut mapped: HashMap<String, String> = HashMap::new();
        let files = manifest.files.clone();
        let sources = manifest.sources.clone();
        map_manifest(&mut manifest, |reference| {
            check_cancel(cancel)?;
            if let Some(url) = sources.get(reference) {
                return Ok(crate::bilibili::source_path(lib, url)?
                    .to_string_lossy()
                    .into_owned());
            }
            let key = files.get(reference).context("备份缺少附件引用")?;
            if let Some(path) = mapped.get(key) {
                return Ok(path.clone());
            }
            ensure!(
                regex::Regex::new(r"^media/[0-9]+\.[A-Za-z0-9]{1,15}$")?.is_match(key),
                "备份附件路径无效"
            );
            let target = stage.join(Path::new(key).file_name().context("附件名称无效")?);
            let entry = zip.by_name(key)?;
            let size = entry.size();
            ensure!(size < MAX_ATTACHMENT_BYTES, "附件过大");
            let mut output = StagedAttachment::new(&target, None)?;
            output.copy(entry, cancel)?;
            output.finish(&target, Some(size), cancel)?;
            let path = target.to_string_lossy().into_owned();
            mapped.insert(key.clone(), path.clone());
            Ok(path)
        })?;
        finish_restore(lib, &mut manifest, &stage, cancel)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}
