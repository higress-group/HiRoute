//! Atomic publication of the complete stopped authority snapshot and recovery instructions.
use super::*;
use crate::migrations::{prepare_storage_root as prepare_owner_directory, validate_owner_file};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;

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
        std::fs::File::open(target)?.sync_all()?;
    } else if metadata.is_file() {
        fs::copy(source, target)?;
        fs::set_permissions(target, metadata.permissions())?;
        std::fs::File::open(target)?.sync_all()?;
    } else {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}
fn excluded(name: &str) -> bool {
    name == "live"
        || name.starts_with("migration-set")
        || name == "startup.lock"
        || name.ends_with(".pid")
        || name.ends_with(".sock")
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), LocalStorageError> {
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
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
        return verify(backup);
    }
    let stage = backup.join("source-staging");
    if stage.exists() {
        // Preserve an interrupted attempt; it is never promoted or reused as complete.
        let mut id = [0u8; 8];
        getrandom::fill(&mut id).map_err(|_| LocalStorageError::Crypto)?;
        let id = CanonicalDigest::of_bytes(&id);
        fs::rename(
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
        if excluded(&name.to_string_lossy()) {
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
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        if metadata.uid() != rustix::process::getuid().as_raw()
                            || metadata.mode() & 0o022 != 0
                        {
                            return Err(LocalStorageError::Permission);
                        }
                    }
                } else {
                    validate_owner_file(&path)?;
                }
                copy_tree(&path, &stage.join(name))?;
                extra.insert(name.to_owned(), path.to_string_lossy().into_owned());
            }
        }
    }
    let extra_paths = extra
        .iter()
        .map(|(name, target)| format!("- {name} → {target}\n"))
        .collect::<String>();
    let text = format!(
        "# HiRoute 手工恢复\n\n备份批次 ID：{}\n\n备份源以本说明所在目录为准，目录改名或归档后仍从这里读取；不要使用其它批次。以下源文件路径均相对于本说明所在目录，目标路径为绝对位置。\n\n1. 完全退出 HiRoute，确认服务、CPA、任务与后台自动重启均已停止。\n2. 将 storage 目录 {} 改名保留；不要改名它的上级目录，备份仍保留在上述位置。恢复将舍弃备份之后的新数据。\n3. 将 storage/ 整个复制回 {}，保留权限。不要逐库覆盖，不要遗留不配套的 WAL/SHM。再按以下源 → 目标路径复制额外文件：\n\n{}\n4. 按下面的完整 App 路径和身份覆盖安装旧包，启动后检查配置、Agent 接入及一次真实请求。\n\n{}\n\n本次只修改三个数据库；该目录同时保存密钥、原生恢复材料及其余自有持久状态。不要将此备份加入诊断导出。\n",
        metadata["batch_id"]
            .as_str()
            .ok_or(LocalStorageError::InvalidData)?,
        root.display(),
        root.display(),
        extra_paths,
        "使用创建备份时的配套完整安装包；本说明不承诺支持其它存储格式。"
    );
    write_private(&stage.join("恢复说明.md"), text.as_bytes())?;
    let mut files = BTreeMap::new();
    collect(&stage, &stage, &mut files)?;
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| LocalStorageError::InvalidData)?
        .as_secs();
    let mut info = metadata.clone();
    info["created_unix_seconds"] = json!(created);
    let manifest = json!({"schema":"hiroute.upgrade-source-backup/v1","complete":true,
        "source_storage":root.to_string_lossy(),"extra_locations":extra,"files":files,"source_info":info});
    write_private(
        &stage.join(MANIFEST),
        &serde_json::to_vec_pretty(&manifest).map_err(|_| LocalStorageError::InvalidData)?,
    )?;
    std::fs::File::open(&stage)?.sync_all()?;
    fs::rename(stage, &source)?;
    std::fs::File::open(backup)?.sync_all()?;
    verify(backup)
}
fn collect(
    root: &Path,
    path: &Path,
    files: &mut BTreeMap<String, String>,
) -> Result<(), LocalStorageError> {
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
            collect(root, &entry.path(), files)?;
        }
    } else if metadata.is_file() {
        files.insert(
            path.strip_prefix(root)
                .map_err(|_| LocalStorageError::InvalidData)?
                .to_string_lossy()
                .into_owned(),
            CanonicalDigest::of_bytes(&fs::read(path)?).to_string(),
        );
    } else {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(())
}
fn manifest(backup: &Path) -> Result<Value, LocalStorageError> {
    validate_owner_file(&backup.join("source").join(MANIFEST))?;
    let value: Value = serde_json::from_slice(&fs::read(backup.join("source").join(MANIFEST))?)
        .map_err(|_| LocalStorageError::InvalidData)?;
    if value["schema"] != "hiroute.upgrade-source-backup/v1"
        || value["complete"] != true
        || value.as_object().is_none_or(|o| o.len() != 6)
    {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(value)
}
pub(super) fn verify(backup: &Path) -> Result<(), LocalStorageError> {
    let value = manifest(backup)?;
    let mut actual = BTreeMap::new();
    collect(&backup.join("source"), &backup.join("source"), &mut actual)?;
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
        collect(root, &entry.path(), &mut actual)?;
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
        if json!(CanonicalDigest::of_bytes(&fs::read(path)?).to_string()) != value["files"][name] {
            return Err(LocalStorageError::InvalidData);
        }
    }
    Ok(())
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
        fs::write(root.join("observation/body"), b"preserved observation").unwrap();
        set_owner_file_permissions(&root.join("observation/body")).unwrap();
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
        for name in ["master-key", "observation/body"] {
            assert_eq!(
                fs::read(root.join(name)).unwrap(),
                fs::read(backup.join("source/storage").join(name)).unwrap()
            );
        }
        let manifest = fs::read(backup.join("source/source-backup.json")).unwrap();
        set.publish_stopped_source(&barrier, &root, &crate::StorageStartupOptions::default())
            .unwrap();
        assert_eq!(
            fs::read(backup.join("source/source-backup.json")).unwrap(),
            manifest
        );
        fs::write(root.join("observation/body"), b"changed").unwrap();
        assert!(verify_unchanged(&root, &backup).is_err());
    }
}
