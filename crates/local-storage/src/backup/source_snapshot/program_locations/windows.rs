use super::*;

pub(super) fn write(stage: &Path, locations: &Value) -> Result<(), LocalStorageError> {
    let locations: hiroute_host_runtime::WindowsDesktopRestore =
        serde_json::from_value(locations.clone()).map_err(|_| LocalStorageError::InvalidData)?;
    locations.validate()?;
    if !stage
        .join("recovery-package/installation/task.xml")
        .is_file()
    {
        return Err(LocalStorageError::InvalidData);
    }
    let program = locations.program.display();
    let cli = locations
        .files
        .values()
        .find(|path| path.file_name().is_some_and(|name| name == "hiroute.exe"))
        .ok_or(LocalStorageError::InvalidData)?
        .display();
    let task = &locations.task_name;
    let mut text = format!(
        r"# Windows program and Task recovery / Windows 程序与任务恢复

Read RECOVERY.md or 恢复说明.md first. Use this exact batch and preserve the newer complete data with its matching installer/program separately. Work in the same ordinary Windows account; administrator access is not required.
先阅读 RECOVERY.md 或恢复说明.md。使用本批次，将新版完整数据与配套安装器/程序另存；在同一个普通 Windows 账户中操作，无需管理员权限。

1. Using the current installed CLI, run `hiroute service autostart disable --output json`, then `hiroute service stop --output json`. Fully quit Desktop and confirm its daemon and CPA have exited. Disable its Desktop login setting before quitting. In Task Scheduler, verify that the recorded `{task}` Task belongs to this account/current HiRoute installation, export it with the newer program, then delete only that stopped registration. Preserve the current program directories and HKCU HiRoute uninstall entry separately. Do not run the product uninstaller or delete user data.
先用当前 CLI 关闭 Task 登录启动并停止服务，退出前也关闭桌面端的登录启动，完全退出后确认守护进程和 CPA 已结束；在任务计划程序中核对本说明记录的 Task 属于当前账户及 HiRoute 安装，将其导出并与新版程序另存，再仅删除这一已停止的注册。另存当前程序目录与当前账户的 HiRoute 卸载注册项，不要运行产品卸载程序或删除用户数据。

If `hiroute` is not on PATH, invoke the installed entry at `{cli}` directly (PowerShell uses `&` before a quoted executable path). If the CLI is damaged, open Task Scheduler under this same account. Verify that the recorded `{task}` Task belongs to this account/current HiRoute installation, export it with the newer program, then stop/disable it and delete only that stopped registration. Confirm its owned processes have exited before copying data. If ownership or process exit cannot be verified, preserve everything and seek support.
若找不到 `hiroute` 命令，请直接运行上面的安装入口（PowerShell 在带引号的程序路径前使用 `&`）。若 CLI 已损坏，在同一账户的任务计划程序中核对本说明记录的 Task 属于当前账户和 HiRoute 安装，将其导出并与新版程序另存，再停止、禁用并仅删除这一已停止的注册；确认其进程均已结束后再复制数据。无法核对归属或退出状态时，请保留现场并寻求支持。

2. Restore the complete original storage group using this batch's recovery guide. Then copy the following entire directories/files, preserving native permissions. The retained native file map includes both CLI/daemon entries, management Skills, package metadata, documentation and licenses.
按本批次说明还原完整旧数据组，再保留原生权限复制下列完整目录和文件。原生文件清单包含 CLI/守护进程入口、管理 Skill、包元数据、文档和许可证。

- recovery-package/program/ → {program}

"
    );
    if let Some(desktop) = &locations.desktop {
        if !desktop.is_absolute() {
            return Err(LocalStorageError::InvalidData);
        }
        text.push_str(&format!(
            "- recovery-package/desktop/ → {}\n",
            desktop.display()
        ));
        if !stage
            .join("recovery-package/installation/uninstall.reg")
            .is_file()
        {
            return Err(LocalStorageError::InvalidData);
        }
        text.push_str(r"
The complete Desktop includes its matching shell, uninstaller and closed inventory. Export the current `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\HiRoute` entry separately. After restoring the old programs, import `recovery-package/installation/uninstall.reg` with Windows `reg.exe import`. This restores the old current-user Apps registration. Do not substitute another account or another revision's installer.
旧桌面包包含配套外壳、卸载程序和完整清单。另行导出当前账户上述 HiRoute 注册项；还原旧程序后，用 Windows `reg.exe import` 导入保留的 uninstall.reg，恢复对应旧版的应用注册。不要换账户或使用其它修订的安装器。

");
    }
    for (name, target) in &locations.files {
        text.push_str(&format!(
            "- recovery-package/{name} → {}\n",
            target.display()
        ));
    }
    let autostart = locations.task_autostart;
    let shell_login = locations.shell_login;
    text.push_str(&format!(r"
3. In Task Scheduler under this same account, import `recovery-package/installation/task.xml` with the exact Task name `{task}`. Verify its account, restored old daemon path, arguments and ordinary-user run level against `installation/windows-background.json`; do not overwrite an unrelated Task or enable elevated privileges. Only after the complete old group and programs are paired, run `hiroute service start --output json` with the restored old CLI. Open the restored Desktop. Its original Desktop login preference was {}. Restore that choice using the matching Desktop's login setting, then run `hiroute service autostart {} --output json` last to retain the Task's independently recorded login choice even if it differed from the shell.
在同一账户的任务计划程序中导入本批次的 task.xml，任务名必须为上面记录的完整名称；对照 windows-background.json 核对账户、已还原旧守护进程的路径、参数及普通用户运行级别，不得覆盖无关任务或开启提升权限。完整旧组与旧程序配套后，用已还原的旧 CLI 启动服务，再打开旧桌面端；先恢复其原先的登录启动选择，最后再运行上述 Task autostart 命令，保留 Task 独立记录的选择（即使它与桌面端原先的选择不同）。

4. Verify `hiroute system status --output json` reports actual Control/Gateway readiness. Confirm the original source, credentials, routing and history, and complete a real request. Task command success alone is insufficient. Post-backup data remains in the separately preserved newer generation.
检查 Control/Gateway 实际就绪、原模型源与凭据、路由和旧历史，并完成一次真实请求；仅 Task 命令成功不代表恢复完成。备份之后的数据保留在另存的新版组中。
", if shell_login { "enabled / 开启" } else { "disabled / 关闭" }, if autostart { "enable" } else { "disable" }));
    write_private(&stage.join("PROGRAM-LOCATIONS.md"), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_recovery_requires_retained_native_registration_and_published_lifecycle() {
        let directory = tempfile::tempdir().unwrap();
        let stage = directory.path();
        let installation = stage.join("recovery-package/installation");
        fs::create_dir_all(&installation).unwrap();
        let locations = hiroute_host_runtime::WindowsDesktopRestore {
            schema: "hiroute.windows-desktop-restore/v1".into(),
            program: stage.join("original-program"),
            desktop: None,
            task_name: format!("HiRoute-{}", "a".repeat(64)),
            task_autostart: true,
            shell_login: false,
            files: BTreeMap::from([
                (
                    "installation/standalone.json".into(),
                    stage.join("standalone.json"),
                ),
                (
                    "installation/windows-background.json".into(),
                    stage.join("windows-background.json"),
                ),
                ("installation/entry-0.bin".into(), stage.join("hiroute.exe")),
            ]),
        };
        let value = serde_json::to_value(locations).unwrap();
        assert!(
            write(stage, &value).is_err(),
            "a JSON definition cannot be imported as a native Task"
        );
        fs::write(
            installation.join("task.xml"),
            b"native registration fixture; not native evidence",
        )
        .unwrap();
        write(stage, &value).unwrap();
        let guide = fs::read_to_string(stage.join("PROGRAM-LOCATIONS.md")).unwrap();
        assert!(guide.contains("recovery-package/installation/task.xml"));
        assert!(guide.contains("hiroute service autostart disable --output json"));
        assert!(guide.contains("hiroute service start --output json"));
        assert!(!guide.contains("hiroute service install"));
        assert!(!guide.contains("hiroute service uninstall"));
    }
}
