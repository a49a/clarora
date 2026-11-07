//! Bounded attachment reads and checks used by the backup formats.
use super::check_cancel;
use crate::process::Cancel;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) fn attachment_stamps(
    files: &BTreeMap<String, PathBuf>,
) -> Result<BTreeMap<PathBuf, (u64, std::time::SystemTime)>> {
    files
        .values()
        .map(|path| {
            let meta = fs::metadata(path)?;
            Ok((path.clone(), (meta.len(), meta.modified()?)))
        })
        .collect()
}
pub(super) fn check_attachments(
    stamps: &BTreeMap<PathBuf, (u64, std::time::SystemTime)>,
    cancel: &Cancel,
) -> Result<()> {
    for (path, stamp) in stamps {
        check_cancel(cancel)?;
        let meta = fs::metadata(path)?;
        ensure!(
            (meta.len(), meta.modified()?) == *stamp,
            "备份期间附件已改变，请重试：{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
    }
    Ok(())
}
pub(super) fn copy_cancellable(
    mut input: impl Read,
    output: &mut impl Write,
    cancel: &Cancel,
) -> Result<u64> {
    let mut bytes = [0; 128 * 1024];
    let mut length = 0;
    loop {
        check_cancel(cancel)?;
        let count = input.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        length += count as u64;
        ensure!(length < MAX_ATTACHMENT_BYTES, "附件过大");
        output.write_all(&bytes[..count])?;
    }
    check_cancel(cancel)?;
    Ok(length)
}
pub(super) fn file_hash(path: &Path, cancel: &Cancel) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 128 * 1024];
    loop {
        check_cancel(cancel)?;
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(hex::encode(hash.finalize()))
}

pub(super) const MAX_ATTACHMENT_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub sha256: String,
    pub length: u64,
    pub key: String,
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub sha256: String,
    pub length: u64,
    pub extension: String,
    pub chunks: Vec<Chunk>,
}
pub fn checksum(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A destination is visible only after all checks and the durable write succeed.
pub(super) struct StagedAttachment<'a> {
    file: tempfile::NamedTempFile,
    length: u64,
    checksum: Option<(&'a str, Sha256)>,
}

impl<'a> StagedAttachment<'a> {
    pub fn new(target: &Path, sha256: Option<&'a str>) -> Result<Self> {
        let parent = target.parent().context("附件落盘路径无效")?;
        Ok(Self {
            file: tempfile::NamedTempFile::new_in(parent)?,
            length: 0,
            checksum: sha256.map(|value| (value, Sha256::new())),
        })
    }

    pub fn write(&mut self, bytes: &[u8], cancel: &Cancel) -> Result<()> {
        check_cancel(cancel)?;
        self.length += bytes.len() as u64;
        ensure!(self.length < MAX_ATTACHMENT_BYTES, "附件过大");
        self.file.write_all(bytes)?;
        if let Some((_, hash)) = &mut self.checksum {
            hash.update(bytes);
        }
        Ok(())
    }

    pub fn copy(&mut self, mut source: impl Read, cancel: &Cancel) -> Result<()> {
        let mut bytes = [0; 128 * 1024];
        loop {
            check_cancel(cancel)?;
            let count = source.read(&mut bytes)?;
            if count == 0 {
                return Ok(());
            }
            self.write(&bytes[..count], cancel)?;
        }
    }

    pub fn finish(self, target: &Path, length: Option<u64>, cancel: &Cancel) -> Result<()> {
        ensure!(
            length.is_none_or(|expected| self.length == expected)
                && self
                    .checksum
                    .is_none_or(|(expected, hash)| hex::encode(hash.finalize()) == expected),
            "附件文件完整性校验失败"
        );
        self.file.as_file().sync_all()?;
        check_cancel(cancel)?;
        self.file.persist_noclobber(target)?;
        Ok(())
    }
}

pub(super) fn materialize_chunks(
    file: &Attachment,
    target: &Path,
    cancel: &Cancel,
    mut fetch: impl FnMut(&Chunk) -> Result<Vec<u8>>,
) -> Result<()> {
    check_cancel(cancel)?;
    ensure!(file.length < MAX_ATTACHMENT_BYTES, "附件过大");
    let mut output = StagedAttachment::new(target, Some(&file.sha256))?;
    for chunk in &file.chunks {
        check_cancel(cancel)?;
        let bytes = fetch(chunk)?;
        ensure!(
            bytes.len() as u64 == chunk.length && checksum(&bytes) == chunk.sha256,
            "附件分块完整性校验失败"
        );
        output.write(&bytes, cancel)?;
    }
    output.finish(target, Some(file.length), cancel)
}
