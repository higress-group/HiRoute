//! Atomic publication of the complete stopped authority snapshot and recovery instructions.
use super::*;
use crate::migrations::{prepare_storage_root as prepare_owner_directory, validate_owner_file};
use serde_json::{Value, json};
use std::collections::BTreeMap;
#[cfg(all(test, target_os = "macos"))]
mod copy_benchmark;
mod program_locations;
mod seal;
#[cfg(windows)]
use std::io::{Read, Write};

const MANIFEST: &str = "source-backup.json";

pub(crate) fn archive_guide(
    backup: &Path,
    archived: &Path,
    batch_id: &str,
) -> Result<(), LocalStorageError> {
    if !backup.join("source").exists() {
        return Ok(());
    }
    verify(backup)?;
    let text = format!(
        "# HiRoute 已完成升级批次定位说明\n\n批次 ID：{batch_id}\n\n恢复源以本说明所在目录/source/ 为准。归档后的完整源路径：{}\n\n请阅读 source/恢复说明.md。旧说明中的固定 migration-set/source 绝对源路径可能已失效，必须改为本说明所在目录/source/，不得使用下一批备份。storage/ 和其它源文件均在该 source/ 下，恢复目标及对应旧完整 App 仍以原说明和 source/source-backup.json 为准。\n\n原备份文件、恢复说明和已哈希清单保持不变。\n",
        archived.join("source").display()
    );
    let directory = hiroute_diagnostics::files::PrivateDir::open_existing(backup)
        .map_err(|_| LocalStorageError::Permission)?;
    let mut stage = directory
        .open_append_or_create("归档说明.tmp")
        .map_err(|_| LocalStorageError::Permission)?;
    stage
        .replace_contents(text.as_bytes())
        .map_err(|_| LocalStorageError::Permission)?;
    stage.sync().map_err(|_| LocalStorageError::Permission)?;
    directory
        .rename_verified("归档说明.tmp", "归档说明.md", stage.identity())
        .map_err(|_| LocalStorageError::Permission)?;
    directory.sync().map_err(|_| LocalStorageError::Permission)
}
fn copy_tree(source: &Path, target: &Path) -> Result<(), LocalStorageError> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(LocalStorageError::Permission);
    }
    if metadata.is_dir() {
        prepare_owner_directory(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
        hiroute_platform::durable::directory_barrier(target)?;
    } else if metadata.is_file() {
        #[cfg(windows)]
        {
            // std::fs::copy/Permissions preserve only the readonly attribute on
            // Windows. Manual recovery must retain the real source owner/DACL.
            // Hold the admitted source through the read and bind all bytes to
            // its exact native permissions; never follow a replaced pathname.
            let mut file = hiroute_platform::windows::security::open_identity(source, false)?;
            if hiroute_platform::windows::security::file_link_count(&file)? != 1 {
                return Err(LocalStorageError::Permission);
            }
            let before = file.metadata()?;
            let permissions = hiroute_platform::file_security::capture(&file)?;
            hiroute_platform::durable::write_windows_atomic_with(target, &permissions, |stage| {
                // A complete database or observation body can be large. Retain
                // one fixed buffer, and never copy beyond the admitted length.
                let mut buffer = zeroize::Zeroizing::new(vec![0_u8; 64 * 1024]);
                let mut copied = 0_u64;
                while copied < before.len() {
                    let count = (before.len() - copied).min(buffer.len() as u64) as usize;
                    file.read_exact(&mut buffer[..count])?;
                    stage.write_all(&buffer[..count])?;
                    copied += count as u64;
                }
                let mut trailing = zeroize::Zeroizing::new([0_u8; 1]);
                let grew = file.read(&mut trailing[..])? != 0;
                let after = file.metadata()?;
                if copied != before.len()
                    || grew
                    || before.len() != after.len()
                    || before.modified()? != after.modified()?
                    || hiroute_platform::windows::security::file_link_count(&file)? != 1
                    || hiroute_platform::file_security::capture(&file)? != permissions
                {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "stopped snapshot source changed during copying",
                    ));
                }
                // Only success allows the platform to flush, verify the exact
                // native descriptor/readonly attribute and publish this stage.
                Ok(())
            })?;
        }
        #[cfg(not(windows))]
        {
            fs::copy(source, target)?;
            fs::set_permissions(target, metadata.permissions())?;
            // This copy is still inside source-staging. Completed directories
            // and the final manifest/stage retain their full device barriers;
            // macOS need not flush the entire drive cache after every body.
            hiroute_platform::durable::sync_unpublished_copy(target)?;
        }
    } else {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}
fn excluded(name: &str) -> bool {
    matches!(
        name,
        "observation.pre-v5-source"
            | "observation.v5-target"
            | "observation.v5-legacy-input"
            | "observation.upgrade.json"
            | "observation-source-backup.json"
    ) || name == "live"
        || name.starts_with("migration-set")
        || name == "startup.lock"
        || name == hiroute_host_runtime::STORAGE_RECOVERY_FILE
        || name.ends_with(".pid")
        || name.ends_with(".sock")
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), LocalStorageError> {
    hiroute_platform::durable::write_private_new(path, bytes)?;
    Ok(())
}
pub(super) fn publish(
    root: &Path,
    backup: &Path,
    metadata: &Value,
    options: &crate::StorageStartupOptions,
) -> Result<(), LocalStorageError> {
    let source = backup.join("source");
    if source.exists() {
        verify(backup)?;
        return publish_locator(
            backup,
            metadata["batch_id"]
                .as_str()
                .ok_or(LocalStorageError::InvalidData)?,
        );
    }
    let stage = backup.join("source-staging");
    if options.preserve_observation_source && stage.join(MANIFEST).exists() {
        return seal::finish(root, backup, &stage, metadata);
    }
    preflight_space(root, Some(backup), options)?;
    if stage.exists() {
        // Never put a moved historical tree into a generic incomplete-copy path.
        if stage.join("storage/observation").exists() {
            return Err(LocalStorageError::InvalidData);
        }
        // Preserve an interrupted attempt; it is never promoted or reused as complete.
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).map_err(|_| LocalStorageError::Crypto)?;
        let id = CanonicalDigest::of_bytes(&id);
        hiroute_platform::durable::rename(
            &stage,
            backup.join(format!("source-incomplete.{}", &id.as_str()[7..23])),
        )?;
    }
    prepare_owner_directory(&stage)?;
    let storage = stage.join("storage");
    prepare_owner_directory(&storage)?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        if excluded(&name.to_string_lossy())
            || (options.preserve_observation_source && name == "observation")
        {
            continue;
        }
        copy_tree(&entry.path(), &storage.join(name))?;
    }
    prepare_owner_directory(&storage.join("live"))?;
    for name in ["control.db", "runtime.db", "secrets.db"] {
        copy_tree(&backup.join(name), &storage.join("live").join(name))?;
    }
    let mut extra = BTreeMap::new();
    if let Some(parent) = root.parent() {
        for (name, path) in [
            (
                "gateway.lkg",
                options
                    .gateway_lkg
                    .clone()
                    .unwrap_or_else(|| parent.join("gateway.lkg")),
            ),
            (
                "gateway-listener.json",
                parent.join("gateway-listener.json"),
            ),
        ] {
            if path.exists() {
                if name == "gateway.lkg" {
                    // Historical cache producers used ordinary 0644 files. This derived,
                    // non-secret cache is kept byte-for-byte for manual restoration only.
                    let metadata = fs::symlink_metadata(&path)?;
                    if !metadata.is_file() {
                        return Err(LocalStorageError::Permission);
                    }
                } else {
                    validate_owner_file(&path)?;
                }
                copy_tree(&path, &stage.join(name))?;
                extra.insert(name.to_owned(), path.to_string_lossy().into_owned());
            }
        }
    }
    let package_identity = if let Some(package) = &options.recovery_package {
        let meta = fs::symlink_metadata(package)?;
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || package.canonicalize()?.starts_with(backup.canonicalize()?)
            || storage_root_conflict(package, root)?
        {
            return Err(LocalStorageError::InvalidData);
        }
        copy_tree(package, &stage.join("recovery-package"))?;
        format!(
            "配套旧包原路径：{}；完整副本位于 recovery-package/，身份以清单中的逐文件 SHA-256 为准。",
            package.display()
        )
    } else {
        "使用创建备份时的配套完整安装包；本说明不承诺支持其它存储格式。保留旧包后再开始安装验证。"
            .to_owned()
    };
    let extra_paths = extra
        .iter()
        .map(|(name, target)| format!("- {name} → {target}\n"))
        .collect::<String>();
    let history = if options.preserve_observation_source {
        "原 Observation 目录已在同一文件系统原地封存；包含原始数据库、WAL/SHM、对象和暂存文件，未逐文件扫描、哈希、转换或压缩。清单仅校验其余恢复材料并绑定封存目录身份，不宣称已逐字节验证历史内容。新版历史和统计从切换后开始。封存仍占用磁盘空间，不受当前观测容量回收影响；它是本机恢复点，不是独立灾备。需要独立备份时，在启动之外复制并校验本目录。恢复前离线检查原始数据库和内容。"
    } else {
        "本次可能转换核心数据库和 Observation 历史；该目录同时保存原始 Observation、密钥、原生恢复材料及其余自有持久状态。"
    };
    let text = format!(
        "# HiRoute 手工恢复\n\n备份批次 ID：{}\n\n备份源以本说明所在目录为准，目录改名或归档后仍从这里读取；不要使用其它批次。以下源文件路径均相对于本说明所在目录，目标路径为绝对位置。\n\n1. 完全退出 HiRoute，确认服务、CPA、任务与后台自动重启均已停止。\n2. 将 storage 目录 {} 改名保留；不要改名它的上级目录，备份仍保留在上述位置。备份之后的新数据必须与配套新版本单独保留，不能混入旧格式恢复。\n3. 将 storage/ 整个复制回 {}，保留权限。不要逐库覆盖，不要遗留不配套的 WAL/SHM。再按以下源 → 目标路径复制额外文件：\n\n{}\n4. 确认完整旧组已还原后，使用下面记录的完整原版程序包。macOS 恢复完整 App；Linux/Windows 按平台安装指南恢复配套程序与服务注册。如存在 PROGRAM-LOCATIONS.md，先按其还原程序、资源和安装记录。启动后检查配置、Agent 接入及一次真实请求。\n\n{}\n\n{history}\n\n不要将此备份加入诊断导出。\n",
        metadata["batch_id"]
            .as_str()
            .ok_or(LocalStorageError::InvalidData)?,
        root.display(),
        root.display(),
        extra_paths,
        package_identity
    );
    write_private(&stage.join("恢复说明.md"), text.as_bytes())?;
    let english_package=options.recovery_package.as_ref().map(|p|format!("Original complete package: {}. The preserved copy is recovery-package/; exact file SHA-256 identities are recorded in source-backup.json.",p.display())).unwrap_or_else(||"Use the exact complete package that produced this backup. Do not substitute a version based only on a label.".into());
    let english = format!(
        "# HiRoute complete-group recovery\n\nBatch: {}\n\nUse this guide's containing directory as the source, including after archival. Never use another batch's source or package.\n\n1. Fully stop HiRoute, its daemon, CPA, tasks and automatic service restarts.\n2. Rename the current storage directory at {} and keep it with its matching newer program. Do not rename the parent containing this backup. Post-backup data must remain separate from the restored old group.\n3. Copy the complete storage/ directory back to {} preserving native permissions. Keep Control, Runtime, Secrets, all keys and Observation DB/WAL/SHM/content together. Never restore only activity.db or mix generations.\n4. Restore the auxiliary files below to their recorded original locations.\n{}\n5. The matching complete previous program is preserved in recovery-package/. For macOS restore the complete App, not individual binaries. For an installed standalone package, use its recorded PROGRAM-LOCATIONS.md instructions to restore the original program, resources and service registration before starting the old service. If no installed-location record exists, follow the matching platform installer guide using the exact preserved package identity; do not guess another release.\n6. Start the matching old program. Verify configuration, Agent access, old history where applicable, actual Control/Gateway readiness and a real request. A successful manager command alone does not establish recovery.\n\n{}\n\nOld Observation history was preserved by a same-filesystem directory transition without an eager full-content verification. Inspect original databases/content offline before restoring. Keep newer data separately. The protected source still occupies disk and is a local recovery point, not an independent disaster-recovery copy. Do not include private backups in diagnostic exports.\n",
        metadata["batch_id"]
            .as_str()
            .ok_or(LocalStorageError::InvalidData)?,
        root.display(),
        root.display(),
        extra_paths,
        english_package
    );
    write_private(&stage.join("RECOVERY.md"), english.as_bytes())?;
    if let Some(package) = &options.recovery_package {
        program_locations::write(&stage, package)?;
    }
    let mut files = BTreeMap::new();
    collect(&stage, &stage, &mut files, false)?;
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| LocalStorageError::InvalidData)?
        .as_secs();
    let mut info = metadata.clone();
    info["created_unix_seconds"] = json!(created);
    let mut manifest = json!({"schema":"hiroute.upgrade-source-backup/v1","complete":true,
        "source_storage":root.to_string_lossy(),"extra_locations":extra,"files":files,"source_info":info});
    if options.preserve_observation_source {
        manifest["schema"] = json!("hiroute.upgrade-source-backup/v2");
        manifest["sealed_observation"] = seal::capture(root, &storage)?;
    }
    write_private(
        &stage.join(MANIFEST),
        &serde_json::to_vec_pretty(&manifest).map_err(|_| LocalStorageError::InvalidData)?,
    )?;
    hiroute_platform::durable::directory_barrier(&stage)?;
    if options.preserve_observation_source {
        return seal::finish(root, backup, &stage, metadata);
    }
    hiroute_platform::durable::rename(stage, &source)?;
    hiroute_platform::durable::directory_barrier(backup)?;
    verify(backup)?;
    publish_locator(
        backup,
        metadata["batch_id"]
            .as_str()
            .ok_or(LocalStorageError::InvalidData)?,
    )
}

/// A bounded directory hint after full verification. It is outside the immutable
/// source and never replaces the authoritative file manifest during recovery.
pub(crate) fn publish_locator(backup: &Path, batch_id: &str) -> Result<(), LocalStorageError> {
    if manifest(backup)?["source_info"]["batch_id"] != batch_id {
        return Err(LocalStorageError::InvalidData);
    }
    let bytes = serde_json::to_vec(&json!({"schema":1,"complete":true,"batch_id":batch_id}))
        .map_err(|_| LocalStorageError::InvalidData)?;
    hiroute_platform::durable::write_private_atomic(&backup.join("source-location.json"), &bytes)?;
    hiroute_platform::durable::directory_barrier(backup)?;
    Ok(())
}

pub(crate) fn preflight_space(
    root: &Path,
    backup: Option<&Path>,
    options: &crate::StorageStartupOptions,
) -> Result<u64, LocalStorageError> {
    if options.preserve_observation_source {
        // The final source will be published under this sibling backup root.
        // Its parent already exists; no directories or history are written here.
        let backup_root = crate::upgrade_backup_root(root)?;
        let parent = backup_root.parent().ok_or(LocalStorageError::InvalidData)?;
        let destination = match fs::symlink_metadata(parent) {
            Ok(_) => parent,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                parent.parent().ok_or(LocalStorageError::InvalidData)?
            }
            Err(e) => return Err(e.into()),
        };
        seal::preflight(root, destination)?;
    }
    fn bytes(path: &Path) -> Result<u64, LocalStorageError> {
        let meta = fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() {
            return Err(LocalStorageError::Permission);
        }
        if meta.is_file() {
            return Ok(meta
                .len()
                .max(fs2::FileExt::allocated_size(&fs::File::open(path)?)?));
        }
        if !meta.is_dir() {
            return Err(LocalStorageError::InvalidData);
        }
        let mut total = 0u64;
        for entry in fs::read_dir(path)? {
            total = total
                .checked_add(bytes(&entry?.path())?)
                .ok_or(LocalStorageError::InvalidData)?;
        }
        Ok(total)
    }
    let mut required = 256_u64 * 1024 * 1024;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !excluded(&entry.file_name().to_string_lossy())
            && !(options.preserve_observation_source && entry.file_name() == "observation")
        {
            required = required.saturating_add(bytes(&entry.path())?);
        }
    }
    for name in ["control.db", "runtime.db", "secrets.db"] {
        let amount = if let Some(backup) = backup {
            bytes(&backup.join(name))?
        } else {
            let live = root.join("live").join(name);
            let wal = live.with_file_name(format!("{name}-wal"));
            let wal_bytes = if wal.try_exists()? { bytes(&wal)? } else { 0 };
            // The provisional check precedes both the coordinated SQL snapshot
            // and its complete-source copy. Startup checks again after snapshot.
            bytes(&live)?.saturating_add(wal_bytes).saturating_mul(2)
        };
        required = required.saturating_add(amount);
    }
    if let Some(package) = &options.recovery_package {
        let meta = fs::symlink_metadata(package)?;
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || (if let Some(backup) = backup {
                package.canonicalize()?.starts_with(backup.canonicalize()?)
            } else {
                false
            })
            || storage_root_conflict(package, root)?
        {
            return Err(LocalStorageError::InvalidData);
        }
        required = required.saturating_add(bytes(package)?);
    }
    let available = fs2::available_space(root)?;
    if available < required {
        let mut recovery = hiroute_host_runtime::StorageRecovery::new(
            hiroute_host_runtime::StorageRecoveryCause::InsufficientSpace,
            hiroute_host_runtime::StorageRecoveryAction::RetryCurrent,
        );
        recovery.storage_space = Some(hiroute_host_runtime::RecoverySpace {
            required_bytes: required,
            available_bytes: available,
        });
        return Err(LocalStorageError::Recovery { recovery: Box::new(recovery),
            source: Box::new(LocalStorageError::InvalidData.at_upgrade_stage("Insufficient space for recovery material and startup headroom; preserve the source and free unrelated space before retrying")) });
    }
    Ok(required)
}
fn collect(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, String>,
    sealed: bool,
) -> Result<(), LocalStorageError> {
    if sealed && path == root.join("storage/observation") {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(LocalStorageError::Permission);
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.path() == root.join(MANIFEST) {
                continue;
            }
            collect(root, &entry.path(), files, sealed)?;
        }
    } else if metadata.is_file() {
        files.insert(
            // The manifest is portable JSON; native Windows separators must
            // not defeat the existing `storage/` and `live/` recovery filters.
            path.strip_prefix(root)
                .map_err(|_| LocalStorageError::InvalidData)?
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/"),
            file_digest(path)?.to_string(),
        );
    } else {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}
fn manifest(backup: &Path) -> Result<Value, LocalStorageError> {
    read_manifest(&backup.join("source"))
}
fn read_manifest(source: &Path) -> Result<Value, LocalStorageError> {
    validate_owner_file(&source.join(MANIFEST))?;
    let value: Value = serde_json::from_slice(&fs::read(source.join(MANIFEST))?)
        .map_err(|_| LocalStorageError::InvalidData)?;
    let fields = match value["schema"].as_str() {
        Some("hiroute.upgrade-source-backup/v1") => 6,
        Some("hiroute.upgrade-source-backup/v2") if value.get("sealed_observation").is_some() => 7,
        _ => return Err(LocalStorageError::InvalidData),
    };
    if value["complete"] != true || value.as_object().is_none_or(|o| o.len() != fields) {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(value)
}
pub(crate) fn verify(backup: &Path) -> Result<(), LocalStorageError> {
    let value = manifest(backup)?;
    verify_files(&backup.join("source"), &value)?;
    if value.get("sealed_observation").is_some() {
        seal::verify_directory(&backup.join("source/storage/observation"), &value)?;
    }
    Ok(())
}
pub(crate) fn verify_sealed(backup: &Path) -> Result<(), LocalStorageError> {
    if manifest(backup)?["schema"] != "hiroute.upgrade-source-backup/v2" {
        return Err(LocalStorageError::InvalidData.at_upgrade_stage(
            "An earlier full-history conversion source exists; preserve it and use its whole-group recovery guide before retrying this upgrade",
        ));
    }
    verify(backup)
}
fn verify_files(source: &Path, value: &Value) -> Result<(), LocalStorageError> {
    let mut actual = BTreeMap::new();
    collect(
        source,
        source,
        &mut actual,
        value.get("sealed_observation").is_some(),
    )?;
    if json!(actual) != value["files"] {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}
pub(crate) fn verify_unchanged(root: &Path, backup: &Path) -> Result<(), LocalStorageError> {
    verify(backup)?;
    let value = manifest(backup)?;
    if value["source_storage"] != root.to_string_lossy().as_ref() {
        return Err(LocalStorageError::InvalidData);
    }
    let mut actual = BTreeMap::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if excluded(&entry.file_name().to_string_lossy()) {
            continue;
        }
        if value.get("sealed_observation").is_some() && entry.file_name() == "observation" {
            // The old generation is bound at its sealed location, not compared
            // with the intentionally empty/new live Observation generation.
            continue;
        }
        collect(root, &entry.path(), &mut actual, false)?;
    }
    let expected = value["files"]
        .as_object()
        .ok_or(LocalStorageError::InvalidData)?
        .iter()
        .filter_map(|(path, digest)| {
            path.strip_prefix("storage/")
                .filter(|p| !p.starts_with("live/"))
                .map(|p| (p.to_owned(), digest.as_str().unwrap_or("").to_owned()))
        })
        .collect::<BTreeMap<_, _>>();
    if actual != expected {
        return Err(LocalStorageError::InvalidData);
    }
    for (name, path) in value["extra_locations"]
        .as_object()
        .ok_or(LocalStorageError::InvalidData)?
    {
        let path = path.as_str().ok_or(LocalStorageError::InvalidData)?;
        if json!(file_digest(Path::new(path))?.to_string()) != value["files"][name] {
            return Err(LocalStorageError::InvalidData);
        }
    }
    Ok(())
}

fn storage_root_conflict(package: &Path, root: &Path) -> Result<bool, LocalStorageError> {
    let package = package.canonicalize()?;
    let root = root.canonicalize()?;
    Ok(package.starts_with(&root) || root.starts_with(&package))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_current_snapshot_preserves_keys_auxiliary_files_and_complete_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("storage");
        let stores = crate::LocalStorageSet::open_for_daemon_startup(&root).unwrap();
        prepare_owner_directory(&root.join("observation")).unwrap();
        hiroute_platform::durable::write_private_new(
            &root.join("observation/body"),
            b"preserved observation",
        )
        .unwrap();
        set_owner_file_permissions(&root.join("observation/body")).unwrap();
        // Cross three complete 64KiB chunks plus a partial tail. Readonly
        // content must retain both bytes and real native security in the copy.
        let chunked = (0..(3 * 64 * 1024 + 17))
            .map(|index| ((index * 37 + 11) & 0xff) as u8)
            .collect::<Vec<_>>();
        let chunked_path = root.join("observation/chunked-readonly");
        hiroute_platform::durable::write_private_new(&chunked_path, &chunked).unwrap();
        let mut readonly = fs::metadata(&chunked_path).unwrap().permissions();
        readonly.set_readonly(true);
        fs::set_permissions(&chunked_path, readonly).unwrap();
        let backup = temporary.path().join("backup");
        let barrier = test_writer_barrier();
        let set = BackupSet::create(
            &barrier,
            &backup,
            &stores.control().connection.borrow(),
            &stores.runtime().connection.borrow(),
            &stores.secrets().connection.borrow(),
        )
        .unwrap();
        set.publish_stopped_source(&barrier, &root, &crate::StorageStartupOptions::default())
            .unwrap();
        verify(&backup).unwrap();
        verify_unchanged(&root, &backup).unwrap();
        for name in [
            "master-key",
            "observation/body",
            "observation/chunked-readonly",
        ] {
            assert_eq!(
                fs::read(root.join(name)).unwrap(),
                fs::read(backup.join("source/storage").join(name)).unwrap()
            );
            #[cfg(windows)]
            {
                let original =
                    hiroute_platform::windows::security::open_identity(&root.join(name), false)
                        .unwrap();
                let copy = hiroute_platform::windows::security::open_identity(
                    &backup.join("source/storage").join(name),
                    false,
                )
                .unwrap();
                assert_eq!(
                    hiroute_platform::file_security::capture(&original).unwrap(),
                    hiroute_platform::file_security::capture(&copy).unwrap(),
                    "the complete source snapshot must retain native recovery permissions",
                );
            }
        }
        assert_eq!(
            fs::read(backup.join("source/storage/observation/chunked-readonly")).unwrap(),
            chunked
        );
        assert!(
            fs::metadata(backup.join("source/storage/observation/chunked-readonly"))
                .unwrap()
                .permissions()
                .readonly()
        );
        let manifest = fs::read(backup.join("source/source-backup.json")).unwrap();
        set.publish_stopped_source(&barrier, &root, &crate::StorageStartupOptions::default())
            .unwrap();
        assert_eq!(
            fs::read(backup.join("source/source-backup.json")).unwrap(),
            manifest
        );
        fs::write(root.join("observation/body"), b"changed").unwrap();
        assert!(verify_unchanged(&root, &backup).is_err());
        #[cfg(windows)]
        for path in [
            chunked_path,
            backup.join("source/storage/observation/chunked-readonly"),
        ] {
            let mut writable = fs::metadata(&path).unwrap().permissions();
            writable.set_readonly(false);
            fs::set_permissions(path, writable).unwrap();
        }
    }
}
