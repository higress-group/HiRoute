import assert from 'node:assert/strict';
import test from 'node:test';
import { storageRecoveryText, startupFailureText } from '../src/storage-recovery.ts';
const report=(cause,action)=>({schema:'hiroute.storage-startup-recovery/v1',cause,action});
test('missing packages and retired or unknown actions never authorize program-only downgrade',()=>{
  for (const language of ['zh','en']) {
    for (const value of [undefined,report('missing_package','preserve_and_diagnose'),report('missing_package','reinstall_original'),report('missing_key','future_action'),{...report('missing_package','preserve_and_diagnose'),schema:'future'}]) {
      const message=storageRecoveryText(value,language,'macos');
      assert.doesNotMatch(message.instructions,language==='zh'?/尚未改变原数据组|重新安装升级前使用的完整原版/:/has not changed the original data group|reinstall the exact complete App/);
      assert.equal(message.canRetry,false);
    }
  }
});
test('Windows does not instruct users to reinstall a macOS App and low space preserves backups',()=>{
  const windows=storageRecoveryText(report('missing_package','preserve_and_diagnose'),'en','windows');
  assert.doesNotMatch(windows.instructions,/App|Settings/);
  const low=storageRecoveryText(report('insufficient_space','retry_current'),'en','windows');
  assert.equal(low.canRetry,true);
  assert.match(low.instructions,/Do not delete upgrade backups/);
});

test('resume requires a portable original batch and measured space stays bounded',()=>{
  for (const id of [undefined,'../foreign','..','',42]) {
    const value={...report('migration_failed','resume_current'),backup_batch:id};
    assert.equal(storageRecoveryText(value,'en','windows').canRetry,false);
  }
  assert.equal(storageRecoveryText({...report('migration_failed','resume_current'),backup_batch:'set_original',backup_name:'observation-format-5'},'en','windows').canRetry,true);
  const low=storageRecoveryText({...report('insufficient_space','retry_current'),storage_space:{required_bytes:1048576000,available_bytes:1048576}},'en','windows');
  assert.match(low.instructions,/1,000 MiB.*1 MiB/);
});

test('verified recovery shows its exact original batch and source, with a usable folder route',()=>{
  for (const language of ['zh','en']) for (const action of ['resume_current','restore_complete_group']) {
    const value={...report('migration_failed',action),backup_batch:'set_original',backup_name:'observation-format-5'};
    const windows=storageRecoveryText(value,language,'windows');
    assert.equal(windows.backupBatch,'set_original');
    assert.equal(windows.backupDirectory,'storage.upgrade-backups/observation-format-5/source/');
    assert.match(windows.locationHint,/RECOVERY\.md.*PROGRAM-LOCATIONS\.md/);
    assert.match(windows.locationHint,language==='zh'?/本机日志目录的上一级/:/parent of the local log folder/);
    const mac=storageRecoveryText(value,language,'macos');
    assert.match(mac.locationHint,language==='zh'?/下方按钮/:/button below/);
  }
});

test('missing or unsafe source names and unknown causes never grant located recovery or resume',()=>{
  for (const name of [undefined,'../foreign','..','',42,'a'.repeat(129)]) {
    const value={...report('migration_failed','resume_current'),backup_batch:'set_original',backup_name:name};
    const message=storageRecoveryText(value,'en','windows');
    assert.equal(message.canRetry,false);assert.equal(message.backupDirectory,undefined);
  }
  for (const cause of ['future_cause','__proto__']) {
    const value={...report(cause,'resume_current'),backup_batch:'set_original',backup_name:'observation-format-5'};
    const message=storageRecoveryText(value,'en','windows');
    assert.equal(message.canRetry,false);assert.equal(message.backupDirectory,undefined);
    assert.match(message.reason,/could not be established/);
  }
});

test('healthy-storage listener failures do not recommend storage recovery',()=>{
  for (const language of ['zh','en']) {
    const value=startupFailureText('DAEMON_GATEWAY_UNAVAILABLE',undefined,language,'macos');
    assert.match(value.reason,language==='zh'?/数据初始化已完成/:/Storage initialization completed/);
    assert.doesNotMatch(value.instructions,language==='zh'?/完整旧数据组/:/restore the complete old group/);
  }
});

test('unverified missing packages give platform-specific repair steps and no blind retry',()=>{
  for (const language of ['zh','en']) {
    const mac=storageRecoveryText(report('missing_package','preserve_and_diagnose'),language,'macos');
    assert.match(mac.instructions,language==='zh'?/尚未确认.*旧版读取/:/Compatibility with the old program has not been established/);
    assert.match(mac.instructions,language==='zh'?/打开诊断目录/:/Open diagnostics/);
    const managed=storageRecoveryText(report('invalid_package','preserve_and_diagnose'),language,'windows');
    assert.match(managed.instructions,language==='zh'?/仅在终端设置环境变量并不够/:/terminal variable alone is insufficient/);
    assert.equal(mac.canRetry,false);
    assert.equal(managed.canRetry,false);
  }
});
