# P5-F449 Service DB Index Admission And Migration

状态：**READY / IMPLEMENTATION ASSIGNED / R449 PENDING**

## Authority

- [`db.md`](../../../../reference/db.md) “Index”
- [`std-surface.md`](../../../../reference/std-surface.md) “Standard Platform Errors”
- [`db-capability-architecture.md`](../../../../architecture/db-capability-architecture.md)
  “Exact Service DB Index Plan”
- [`package-service-contract-deployment.md`](../../../../architecture/package-service-contract-deployment.md)
  §11
- [`runtime-deployment-topology.md`](../../../../architecture/runtime-deployment-topology.md)
  “Activation 与 Generation”
- [`P5-F446C-activation-runtime-service-db.md`](P5-F446C-activation-runtime-service-db.md)
- [`P5-F448-activation-owner-switch-atomic-rebind.md`](P5-F448-activation-owner-switch-atomic-rebind.md)

本文只拆实现owner、迁移顺序和证据，不新增第二套index或数据保留语义。

## Finding

现有File IR已经记录普通/unique index，但Runtime从未把声明协调成Mongo index；实际service DB只有`_id_`。
因此源码中的普通索引没有性能保证，unique约束也没有生效。现有partial index还把raw source expression AST
传入artifact/runtime，却没有可执行的typed predicate contract；这是表面支持、实际无效的路径。

F446/R446不能在该缺口存在时收尾。F449硬切为：

- Runtime在candidate prepare和cold recovery的prepared ACK前协调完整exact service DB index plan；
- 普通与unique index成为真实约束，partial index在compiler边界明确拒绝；
- 当前本机旧库只迁移仍有价值的定义、设置和credential/key/upstream数据，聊天运行数据丢弃；
- 旧库及迁移前备份保留，不做不可恢复删除。

## Implementation Ownership

### Compiler/artifact/std checkpoint

- compiler遇到index `where`立即给出明确unsupported诊断；
- 从File IR、runtime projection、linked metadata、identity/golden和fixture删除raw `where` Source AST字段，
  不保留兼容reader/writer或`null`占位；
- 普通/unique index只携带canonical logical name、ordered field paths、direction和unique；
- index path复用既有DB field policy，encrypted、recoverable-envelope内部、动态shape及不可查询path保持拒绝；
- 新增公开`std.db.ConstraintError { target, message, retryable }`，unique duplicate固定输出
  `target: "std.db"`、`message: "database constraint violated"`、`retryable: false`；
- duplicate mapping不得包含Mongo error/code、database、collection、physical index、key pattern/value或
  原始document。

### Runtime plan/admission checkpoint

- 从exact provider DB metadata按
  `(trusted storage domain, environment, serviceId)`构造完整plan；
- candidate含同service多version时先在内存中合并验证，再执行任何Mongo mutation：同logical collection/index
  同定义去重，不同定义失败，不同名字取并集；
- physical index name系统编码
  `(packageId, logical collection identity, logical index identity)`，不含build/version/alias/edge/replica；
- 受管index固定simple/binary collation，field path到Mongo key复用唯一physical field mapper；
- prepare与cold recovery都在activation context publish/prepared ACK前协调；
- missing index additive create，exact definition通过，changed或removed managed index失败；
- unmanaged index保留；Mongo `_id_`忽略；带系统reserved marker但不能合法decode/归属的index必须失败，
  不能伪装unmanaged；
- 每replica执行同一plan。并发exact create幂等，创建后复读definition；冲突、部分创建或复读不一致拒绝
  candidate，不能发布半prepared context；
- activation不自动drop、rename或rebuild index。需要change/remove时先执行显式migration；
- unique duplicate在业务write映射为可catch的`std.db.ConstraintError`；prepare/recovery中的duplicate只
  形成脱敏activation rejection。

### Data migration owner

本机迁移严格按以下顺序执行，任何一步失败都不activation：

1. 停止旧库和目标库的业务写入，记录受信source/target storage domain、environment、service identity与
   physical collection mapping；
2. 分别对旧库和当前目标库做完整备份并校验可读；当前目标库中的本轮rollout/test数据可以在备份后清空；
3. 按candidate全部unique定义审计旧库duplicate，输出只含collection logical identity、index logical
   identity和count的脱敏报告，不输出key/value；
4. 在空目标库按F449 canonical plan建立并复读普通/unique index；
5. filtered copy仅保留下列数据：
   - Agine Agent定义和设置；
   - Agine provider及API key；
   - Codex Relay API key；
   - ChatGPT upstream/source定义；
   - ChatGPT Plan OAuth credential。
6. 不复制chat/message/thread/session/run/tool call/tool result/interaction/relay interaction及其它聊天执行
   历史；无法明确归类的document不自动复制，留在旧库和备份中；
7. selected encrypted field先按明确source context读取v1 envelope，再按target
   environment/service/physical collection/field/record AAD写成v2；不允许目标侧明文fallback、原样保留
   v1或在日志/临时JSON中落plaintext；
8. 校验filtered count、record shape、unique约束、v2 envelope、用当前keyring读取credential，以及Agent/
   provider/upstream最小业务读取；
9. 最后才activation新candidate。旧数据库和两份备份继续保留，不drop、不覆盖。

迁移工具必须支持dry-run、重复运行检测和清晰的source/target确认；它不能猜stable端口、database name或
collection name，也不能把历史chat schema强行升级成当前shape。

## Integration And Evidence

- 开发agent在自己的worktree只做格式、静态检查、测试代码审阅和`git diff --check`，不运行Rust编译；
- compiler/artifact/std checkpoint先合流，再让Runtime与migration consumer消费；
- 所有候选进入一个integration branch后，由唯一gate owner编译一次并运行受影响聚焦测试；
- 动态Mongo证据至少覆盖empty create、exact restart、multi-version merge、unique duplicate、
  changed/removed managed、unmanaged preserved、`_id_` ignored、reserved malformed、two-replica race、
  prepare rejection不替换active和cold recovery；
- 反向搜索raw index `where` AST、runtime `where_filter`、自动drop/rebuild和Mongo detail leakage为零；
- [`P5-R449-service-db-index-acceptance.md`](P5-R449-service-db-index-acceptance.md)独立PASS前，F446、
  F448和R446继续保持pending，不能用迁移成功替代实现验收。

## Out Of Scope

- 不设计partial index typed predicate、locale-aware/case-insensitive collation或在线index migration；
- 不自动修复duplicate，不选择保留哪条冲突record；
- 不迁移历史聊天运行数据，不删除旧库/备份；
- 不改变service DB identity、collection identity、DB query surface或activation CAS owner；
- 不push，不操作production远端数据。
