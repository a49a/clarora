//! Portable backups and cloud synchronization. Object storage transport lives separately.
use crate::data::schema::{TABLES, table_spec};
use crate::data::{Library, Row, id, insert, text};
use crate::object_storage::check_cancel;
use crate::process::Cancel;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
};

mod archive;
mod attachments;
pub mod cloud;
mod legacy;
mod manifest;
mod restore;
pub mod sync;

pub use crate::object_storage::{
    Storage, StorageConfig, StorageProfile, UploadProgress, UploadStage, list_profiles, profiles,
    select_profile,
};
pub use archive::{export_local, export_local_cancellable, import_local, import_local_cancellable};
pub use cloud::{BackupVersion, CleanupPlan, CleanupResult, Retention};
pub use manifest::{
    BookAttachment, ContextBinding, LibraryState, Manifest, SubtitleAttachment, collect, validate,
};
pub use restore::{import_sqlite, merge};
pub use sync::SyncResult;

use attachments::{attachment_stamps, check_attachments, file_hash};
use manifest::{
    collect_cancellable, hash, map_manifest, material_context, safe_setting, source_paths,
    valid_extension,
};
use restore::{finish_restore, setting_in};
