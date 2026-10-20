//! Exact installed program/registration locations for the complete source's guide.
use super::*;
mod windows;

pub(super) fn write(stage: &Path, package: &Path) -> Result<(), LocalStorageError> {
    let locations = package.join("restore-locations.json");
    if locations.exists() {
        validate_owner_file(&locations)?;
        if fs::metadata(&locations)?.len() > 16 * 1024 {
            return Err(LocalStorageError::InvalidData);
        }
        let locations: Value = serde_json::from_slice(&fs::read(locations)?)
            .map_err(|_| LocalStorageError::InvalidData)?;
        if locations["schema"] == "hiroute.windows-desktop-restore/v1" {
            return windows::write(stage, &locations);
        }
        if locations["schema"] != "hiroute.standalone-program-restore/v1" {
            return Err(LocalStorageError::InvalidData);
        }
        let mut steps = String::from(
            "# Standalone program locations / 独立安装原位置\n\nStop the service and restore the complete old storage group first. Copy the following retained directories/files, preserving permissions, then reload the native service manager and start the exact restored service. Do not run a newer installer over the restored old group. / 先停止服务并还原完整旧数据组，再保留权限复制下列目录和文件，重新加载服务管理器并启动配套旧服务。不要用新版安装器覆盖旧组。\n\n",
        );
        for name in ["program", "resources"] {
            let target = locations[name]
                .as_str()
                .ok_or(LocalStorageError::InvalidData)?;
            if !Path::new(target).is_absolute() {
                return Err(LocalStorageError::InvalidData);
            }
            steps.push_str(&format!("- recovery-package/{name}/ → {target}\n"));
        }
        for (name, target) in locations["files"]
            .as_object()
            .ok_or(LocalStorageError::InvalidData)?
        {
            let target = target.as_str().ok_or(LocalStorageError::InvalidData)?;
            if !name.starts_with("installation/")
                || name.contains("..")
                || !Path::new(target).is_absolute()
            {
                return Err(LocalStorageError::InvalidData);
            }
            steps.push_str(&format!("- recovery-package/{name} → {target}\n"));
        }
        if let Some(links) = locations.get("links") {
            let links = links.as_object().ok_or(LocalStorageError::InvalidData)?;
            if links.len() != 2 {
                return Err(LocalStorageError::InvalidData);
            }
            steps.push_str("\nRecreate the following symbolic links after restoring the old directories. Replace only these installer-owned entries; preserve a foreign entry and seek support. / 还原旧目录后，重建以下符号链接。只替换这些安装器自有入口；遇到外来文件请保留并寻求支持。\n\n");
            let program = Path::new(
                locations["program"]
                    .as_str()
                    .ok_or(LocalStorageError::InvalidData)?,
            );
            for (link, target) in links {
                let link = Path::new(link);
                let target = Path::new(target.as_str().ok_or(LocalStorageError::InvalidData)?);
                if !link.is_absolute()
                    || !matches!(
                        link.file_name().and_then(|n| n.to_str()),
                        Some("hiroute" | "hirouted")
                    )
                    || target
                        != program.join(link.file_name().ok_or(LocalStorageError::InvalidData)?)
                {
                    return Err(LocalStorageError::InvalidData);
                }
                steps.push_str(&format!("- {} → {}\n", link.display(), target.display()));
            }
        }
        write_private(&stage.join("PROGRAM-LOCATIONS.md"), steps.as_bytes())?;
    }
    Ok(())
}
