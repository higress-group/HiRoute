// Presentation only: the storage owner supplies the recovery disposition.
export type StorageRecovery = {
  schema: 'hiroute.storage-startup-recovery/v1';
  cause: 'missing_package' | 'invalid_package' | 'insufficient_space' | 'access_denied' | 'locked' | 'missing_key' | 'unsupported_format' | 'namespace_unavailable' | 'migration_failed' | 'unknown_state';
  action: 'retry_current' | 'preserve_and_diagnose' | 'resume_current' | 'restore_complete_group';
  backup_batch?: string;
  backup_name?: string;
  storage_space?: { required_bytes: number; available_bytes: number };
};
type RecoveryText = { reason: string; instructions: string; canRetry: boolean; backupBatch?: string; backupDirectory?: string; locationHint?: string };
export function storageRecoveryText(value: unknown, language: 'zh' | 'en', platform: string): RecoveryText {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  const record = value && typeof value === 'object' ? value as Partial<StorageRecovery> : null;
  const trusted = record?.schema === 'hiroute.storage-startup-recovery/v1';
  const cause = trusted ? record.cause : undefined;
  const reasons: Record<string, string> = {
    missing_package: text('缺少配套旧版本的完整恢复包，升级已停止。', 'The upgrade stopped because the matching complete previous package is missing.'),
    invalid_package: text('旧版本恢复包或升级交接记录无法通过校验。', 'The previous recovery package or installation handoff could not be verified.'),
    insufficient_space: text('数据所在磁盘的恢复材料与启动空间不足。', 'The storage volume has insufficient space for recovery material and startup.'),
    access_denied: text('无法安全访问所需文件，请检查占用、权限与所有者。', 'Required files could not be accessed safely. Check file use, permissions and ownership.'),
    locked: text('另一个 HiRoute 实例正在使用这组数据。', 'Another HiRoute instance is using this data group.'),
    missing_key: text('原数据所需的密钥无法读取或校验。', 'A key required by the original data is unavailable or invalid.'),
    unsupported_format: text('这组数据的格式没有经过验证的升级路径。', 'This data format has no verified upgrade path.'),
    namespace_unavailable: text('历史目录无法在同一文件系统内安全封存。请保留材料，检查原历史目录和恢复目标所在磁盘及占用情况。', 'The history directory cannot be sealed safely on the same filesystem. Keep all material and check the history and recovery destination volumes and file use.'),
    migration_failed: text('数据升级未完成，需要核对保留的恢复材料。', 'The storage upgrade did not complete. Check the preserved recovery material.'),
  };
  const knownCause = cause === 'unknown_state' || !!cause && Object.hasOwn(reasons, cause);
  const reason = cause && Object.hasOwn(reasons, cause) ? reasons[cause] : text('无法确认这组数据的升级状态。', 'The storage upgrade state could not be established.');
  let instructions = text('请保留所有数据版本、密钥、完整安装包和升级记录。不要直接降级、删除数据或反复重试。查看启动诊断和对应批次的完整恢复说明；整组还原前，先另存新版数据及其配套程序。缺少密钥或完整备份时，请保留材料并寻求支持。', 'Keep every data generation, key, complete package and upgrade journal. Do not downgrade, delete data or retry blindly. Inspect startup diagnostics and the matching complete-group recovery guide. Preserve newer data and its matching program separately before restoration. Missing keys or complete backups may require support.');
  const portable = (name: unknown): name is string => typeof name === 'string' && /^[A-Za-z0-9._-]{1,128}$/.test(name) && !name.includes('..');
  const batch = trusted && knownCause && portable(record.backup_batch) && portable(record.backup_name);
  const located = batch && ['resume_current', 'restore_complete_group'].includes(record.action ?? '');
  if (trusted && ['resume_current', 'restore_complete_group'].includes(record.action ?? '') && batch) {
    instructions = record.action === 'resume_current'
      ? text('已识别并校验原升级批次的恢复材料。请保留所有数据，重新打开当前版本继续该批次；不要运行旧版程序。若仍无法继续，请按该批次的完整恢复说明处理，并先另存新版数据及配套程序。', 'The original upgrade batch and its recovery material were verified. Keep all data and reopen the current version to continue that batch. Do not run an older program. If it cannot continue, follow this batch’s complete recovery guide, first preserving newer data and its matching program separately.')
      : text('请停止 HiRoute，先另存新版数据及其配套程序，再按已校验批次的恢复说明，还原完整旧数据组及配套旧程序。不要逐库覆盖或混用不同批次。', 'Stop HiRoute and preserve newer data with its matching program separately. Follow the verified batch’s recovery guide to restore the complete old group and matching old program. Never replace individual databases or mix batches.');
  } else if (cause === 'missing_package' || cause === 'invalid_package') {
    instructions = platform === 'macos'
      ? text('请保留所有数据、密钥、升级记录和升级前使用的完整 App。尚未确认原数据组可由旧版读取，请勿直接降级或覆盖数据。打开诊断目录，联系支持核对配套恢复包和受支持的升级路径；缺少原版完整 App 或必要材料时，无法保证恢复服务。', 'Keep all data, keys, upgrade journals and the complete App used before this upgrade. Compatibility with the old program has not been established; do not downgrade or overwrite data. Open diagnostics and contact support to verify the matching recovery package and supported upgrade route. Recovery cannot be guaranteed without the original complete App and required material.')
      : text('请保留全部数据和升级记录，通过受支持的安装流程补齐并校验配套旧版本完整包。后台服务必须实际收到恢复包信息；仅在终端设置环境变量并不够。查看安装指南和启动诊断；请勿直接降级或覆盖数据。原包或必要材料无法取得时，请联系支持。', 'Keep all data and upgrade journals. Supply and verify the complete matching previous package through the supported installation workflow. The actual background service must receive the recovery input; setting a terminal variable alone is insufficient. Follow the installation guide and inspect startup diagnostics. Do not downgrade or overwrite data. Contact support if the original package or required material is unavailable.');
  } else if (cause === 'insufficient_space') {
    instructions = text('请释放数据所在磁盘上的无关文件，保留原数据、封存历史和恢复包。空间足够后，重新打开当前版本继续恢复；不要删除升级备份。', 'Free unrelated files on the storage volume, keeping original data, sealed history and recovery packages. Once enough space is available, reopen the current version to continue recovery. Do not delete upgrade backups.');
  } else if (cause === 'access_denied' || cause === 'locked') {
    instructions = text('请关闭其他 HiRoute 实例并解决相关文件占用或访问问题，再重新打开当前版本。保留所有数据和升级记录，不要批量修改权限或清空目录。', 'Close other HiRoute instances and resolve the file-use or access issue, then reopen the current version. Keep all data and upgrade journals. Do not change permissions recursively or empty directories.');
  }
  const space = trusted && record.storage_space;
  if (cause === 'insufficient_space' && space && Number.isSafeInteger(space.required_bytes) && Number.isSafeInteger(space.available_bytes) && space.required_bytes >= 0 && space.available_bytes >= 0) {
    const mb = (bytes: number) => `${Math.ceil(bytes / 1048576).toLocaleString(language === 'zh' ? 'zh-CN' : 'en-US')} MiB`;
    instructions += text(` 数据所在磁盘预计需要 ${mb(space.required_bytes)}，当前可用 ${mb(space.available_bytes)}。`, ` The storage volume requires approximately ${mb(space.required_bytes)}; ${mb(space.available_bytes)} is available.`);
  }
  return { reason, instructions,
    backupBatch: located ? record.backup_batch : undefined,
    backupDirectory: located ? `storage.upgrade-backups/${record.backup_name}/source/` : undefined,
    locationHint: located ? platform === 'windows'
      ? text('在下方本机日志目录的上一级，进入上述备份目录。先读 RECOVERY.md 或恢复说明.md，再按需阅读 PROGRAM-LOCATIONS.md。', 'From the parent of the local log folder below, open this backup directory. Read RECOVERY.md (or 恢复说明.md), then PROGRAM-LOCATIONS.md where present.')
      : text('通过下方按钮打开已保留的恢复材料，核对批次后阅读 RECOVERY.md 或恢复说明.md；不要使用其它批次。', 'Open the preserved recovery material with the button below. Match this batch and read RECOVERY.md or 恢复说明.md; do not use another batch.')
      : undefined,
    canRetry: trusted && (record.action === 'resume_current' && batch || record.action === 'retry_current' && ['insufficient_space', 'access_denied', 'locked'].includes(cause ?? '')) };
}

export function startupFailureText(code: string | undefined | null, recovery: unknown, language: 'zh' | 'en', platform: string): RecoveryText {
  const text = (zh: string, en: string) => language === 'zh' ? zh : en;
  if (recovery || ['DAEMON_STORAGE_UNREADABLE', 'DAEMON_UPGRADE_SOURCE_UNSUPPORTED', 'DAEMON_UPGRADE_STORAGE_FAILED'].includes(code ?? '')) {
    return storageRecoveryText(recovery, language, platform);
  }
  const reasons: Record<string, string> = {
    DAEMON_DEPENDENCY_UNAVAILABLE: text('本机服务所需的配套组件无法启动。', 'A required bundled service component could not start.'),
    DAEMON_GATEWAY_UNAVAILABLE: text('数据初始化已完成，但 Gateway 监听未就绪。', 'Storage initialization completed, but the Gateway listener is unavailable.'),
    DAEMON_CONTROL_UNAVAILABLE: text('本机控制接口无法启动。', 'The local control interface could not start.'),
    DAEMON_RELEASE_INVALID: text('当前完整程序的发布信息无法通过校验。', 'The complete program’s release identity could not be verified.'),
    DAEMON_START_CANCELLED: text('本次服务启动已取消。', 'This service startup was cancelled.'),
  };
  return {
    reason: reasons[code ?? ''] ?? text('本机服务尚未就绪，请查看启动诊断。', 'The local service is unavailable. Inspect startup diagnostics.'),
    instructions: code === 'DAEMON_GATEWAY_UNAVAILABLE'
      ? text('请检查配置的监听地址、端口及占用情况，再重新启动当前版本。此提示不要求还原数据库；请保留现有数据与恢复材料。', 'Check the configured listener address, port and file use, then reopen the current version. This failure does not require database restoration. Keep existing data and recovery material.')
      : text('请保留数据和恢复材料，查看本机诊断中的具体原因。检查当前完整安装包及其配套组件；需要修复程序时使用同一版本的完整安装包。此提示不授权降级或还原数据库。', 'Keep data and recovery material and inspect the local startup diagnostics. Check the complete current package and its bundled components; use the same complete version for program repair. This failure does not authorize downgrade or database restoration.'),
    canRetry: code === 'DAEMON_START_CANCELLED',
  };
}
