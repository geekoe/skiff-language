# P5-R449 Service DB Index Acceptance

## Role

在F449 exact integration candidate上做独立只读验收。验收agent不修代码、不执行数据删除，也不把“Mongo里
碰巧已有索引”当作Runtime admission正确的替代证据。

## Authority

- [`P5-F449-service-db-index-admission-and-migration.md`](P5-F449-service-db-index-admission-and-migration.md)
- [`db-capability-architecture.md`](../../../../architecture/db-capability-architecture.md)
  “Exact Service DB Index Plan”
- [`db.md`](../../../../reference/db.md) “Index”
- [`runtime-deployment-topology.md`](../../../../architecture/runtime-deployment-topology.md)
  “Activation 与 Generation”

## Acceptance Matrix

| 场景 | 必须观察到 |
| --- | --- |
| compiler读取普通/unique index | 只产出logical identity、ordered path/direction和unique；field policy与query/order共用 |
| compiler读取index `where` | 明确unsupported并停止；artifact/runtime无raw AST、`where`/`whereFilter`或null占位 |
| empty DB prepare | prepared ACK前创建全部受管index并复读exact definition；固定simple/binary collation |
| exact candidate重试或cold recovery | 不重建、不报冲突；同一plan幂等通过 |
| 同service多version | mutation前先合并完整plan；相同定义去重，不同名字取并集，同logical identity不同定义拒绝 |
| 两个Runtime replica并发prepare | exact create幂等收敛；两者只在复读一致后prepared |
| missing managed index | additive create；不会drop其它index |
| changed/removed managed index | fail closed，active generation不变；不会自动drop/rebuild/rename |
| unmanaged index与Mongo `_id_` | 原样保留且不进入managed removed比较 |
| reserved malformed/错误owner physical name | fail closed，不能降级成unmanaged |
| 历史duplicate阻止unique create | sanitized、non-retryable constraint rejection，无Mongo/DB/key/value细节 |
| 业务write违反unique | 可catch `std.db.ConstraintError`，固定target/message/retryable，stack按普通错误规则存在 |
| partial create或复读不一致 | candidate rejected，未发布半构造activation context |
| migration dry-run | 明确source/target，报告保留/丢弃分类和duplicate count，不输出plaintext或业务key |
| filtered migration | 只保留Agent定义/设置及credential/key/upstream；聊天/session/run/tool/interaction不进入目标 |
| encrypted migration | v1只按source context读取，目标全部为v2且可用当前keyring读取；无plaintext fallback/temp/log |
| activation顺序 | stop-write/backup/audit/index/copy/re-encrypt/verify全部完成后才activation |
| rollback资产 | 旧库与旧/新库迁移前备份均保留、可读且未覆盖 |

## Static And Gate Evidence

- managed physical name只由一个canonical encoder/decoder owner生成并识别；
- field path physical mapping只有一个owner，index没有独立dot-path解释器；
- Runtime prepare与cold recovery调用同一exact plan coordinator，prepared ACK和context publish不能绕过；
- ordinary Mongo error不错误映射成constraint，constraint也不泄漏backend detail；
- F449聚焦Mongo动态测试、受影响Rust/Node gate和`git diff --check`来自同一exact commit/tree；
- Rust编译只由integration gate owner在合流后执行一次；
- 数据迁移receipt记录备份身份、过滤类别、count、v1→v2和验证结果，但不记录secret值；
- F448、F446与R446仍保持pending；R449 PASS只解除index/迁移前置，不单独宣称Phase 05完成。

第一行输出`PASS`或`FAIL`。FAIL必须给出exact commit/tree、失败场景、唯一production owner和最小修复边界；
不得在验收worktree修改候选。
