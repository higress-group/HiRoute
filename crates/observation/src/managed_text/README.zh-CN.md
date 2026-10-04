# 受管正文内部接口

[English](README.md)

入口是 `hiroute_observation::managed_text` 的类型和
`LocalObservationStore::managed_text_*` 方法。它们不是公开 RPC，也不签发权限。
Application / 委派执行服务必须用已认证的 task/run 构造 `ManagedTextScope`，
每次读检查内容许可；不能从 Worker payload、环境变量或引用本身取得权限。

## 写入和重试

1. 委派执行服务先创建可信 task/run，调用 `managed_text_put(input, now_ms)`。
2. `source_event_id + source_revision + scope` 标识同一事件；重复调用返回同一引用。
   改目的、原始时间或导入来源会产生 `Conflict`，不会刷新七天期限。
3. `append` 以从 0 开始的连续 ordinal 写块，每块 1～65536 bytes。
   同块同 bytes 可重试，不同 bytes 冲突。待写块先登记，完整落盘后才对读者发布。
4. `finish` 提交准确的已完成块数，取得 Complete 引用，再由委派执行服务保存业务引用。
   跨库无分布式事务；业务提交失败重用原引用，内容失败由委派执行服务保留真实任务结果和缺失状态。

`original_created_at_ms` 和 `now_ms` 来自可信生产事件/服务端时钟。
历史导入须给出仍可见的原引用，原截止时间必须相同。没有可证明来源时，
调用方不能把历史冒充新事件。继续旧 run 时应复用旧正文引用及其 scope，
由上层明确授权跨 run 读取；本接口不接受用新 run 身份重新标记原引用。

## 读取和继续

`resolve` 总是读取持久化的当前可见性，不采信引用中缓存的 state/generation。
`read` 每次最多 16 块（1 MiB），返回 `next_chunk`，在文件读取后复核 generation。
读者遇到 Stale 时先重新 resolve；Deleted/Expired/Unavailable 不能回退到本地缓存。
Complete 表示发布完成，不保证磁盘永久无损；正文实际消费还须成功 read。
Storage 错误不可当成空正文或用于证明“历史可继续”。

## 删除、过期和原生缓存

Application 以独立管理授权生成 preview 并取得精确确认后调用 apply。
preview 的 cutoff/count/generation 过时则 Stale；确认之前零删除。
apply 的事务先撤可见性并登记持久化清理记录，然后由 `gc` 每批最多 200 个块回收。
cutoff 以内的迟到事件不会恢复正文；确证 cutoff 之后的新事件可写入。

委派执行服务用 `pending_native_cleanup(scope, after_generation, limit)` 恢复未确认的原生缓存清理，
仅处理同 scope、cutoff 内的 HiRoute 受管历史，实际成功后调用 `native_gc_ack`。
通知可以丢失，主动查询和继续仍须检查当前引用。apply 重试返回当前两个 GC 状态；
任一 pending 就不能宣称清理全部完成。`expire_scope` 供 retention worker 调度，
采用每个事件七天截止时间，并使用同一原生清理协议。

生产 maintenance 已调度过期 scope 发现和分批对象 GC。
Application 授权和会话删除已通过
[`ObservationControl`](../../../application/src/observation_query/control/v2.rs)
及[统一保留期处理](../store/unified_retention.rs)接入。
Worker 原生缓存清理由 daemon 的[维护消费者](../../../daemon/src/control/runtime/delegation_maintenance.rs)
领取待办并管理确认状态，实际成功后才确认完成；有界文件删除交给
[native_cleanup.rs](../../../daemon/src/delegation/native_cleanup.rs)。
本存储接口测试及[观察夹具检查](../../../../tools/product-e2e/tests/local_observation.rs)不能代替生产入口证据；
请查阅[已安装 CLI 管理闭环](../../../daemon/tests/publication_process.rs)及
[Worker 读取/继续场景](../../../../tools/product-e2e/tests/worker_read.rs)中的验证范围和限制。
