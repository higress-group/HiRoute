# 消息可靠性研究：Kafka 与 SQL 业务一致性决策复核

为六系统研究资料包提交一份简短决策备忘录，仅分析下述 Kafka 与 SQL 账本设计。使用本目录冻结的官方摘录与显式假设，不可联网；不得从单个系统宣传语推出跨系统保证。

交付 decision.json：顶层为 {"memos":[...]}，只含 M2。包含 id、answers（下列键的机器可读答案）、reason（中文简短论证，保留前提）、counterexample（对不成立的方案给最短故障时序）、repair（边界内的最小修复建议）、citations（[{"source":"kafka","start":12,"end":16}] 等，使用原始 L 行号）。无需最低字数，不要重复原文；建议验证标为尚未执行。

## M2：Kafka 与独立 SQL 账本的故障原子性

Kafka 已读到业务事件 e，后续偏移提交均在独立 Kafka 事务中，事务含输出 topic 和输入偏移；SQL 与 Kafka 没有共同事务。SQL 具有永久唯一事件键、串行化事务和可靠持久性。崩溃可发生在任何步骤之间，Kafka 消息会一直保留到恢复；无限期停机不计入本题，最终存在一次无故障重试。只讨论 SQL 余额更新，不包括邮件或外部扣款。
候选 A：先 SQL 提交 inbox(e)，再在另一 SQL 事务中余额 +10，最后 Kafka 提交；重放时若 inbox 已有 e 则跳过余额更新并推进 Kafka。
候选 B：一个 SQL 事务内执行 INSERT inbox(e) ON CONFLICT DO NOTHING；无论 INSERT 是否插入，都余额 +10；随后 Kafka 提交。
候选 C：一个 SQL 事务内执行 INSERT inbox(e) ON CONFLICT DO NOTHING；仅在本次 INSERT 实际插入时余额 +10；随后 Kafka 提交。
需要对每个方案分别判断：任意允许崩溃恢复轨迹下，余额增加次数不会超过一次（safety），以及最终至少增加一次（liveness）。
answers：a_safety、a_liveness、b_safety、b_liveness、c_safety、c_liveness，均布尔值。
另答 c_requires_distributed_transaction_for_sql_once（布尔值）：在已给定全部前提下，C 是否仍必须引入 SQL/Kafka 两阶段提交，才能让 SQL 余额最终恰好增加一次？
reason 须分清 SQL 恰好一次与跨系统所有输出原子可见。不要将 C 的局部结论扩展为任意外部系统的一次副作用保证。

