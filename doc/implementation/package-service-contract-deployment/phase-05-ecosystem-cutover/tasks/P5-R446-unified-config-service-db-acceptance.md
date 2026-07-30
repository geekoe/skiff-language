# P5-R446 Unified Config And Service DB Acceptance

## Role

独立只读验收F446A–D、F447、F448与F449 exact integration candidate。R447、R448和R449必须已经各自在
同一候选上PASS。先核验共同验收矩阵与跨仓库commit/tree，再按风险选择聚焦probe；同一代码状态已有昂贵完整gate时
不机械重跑。

## Blocking Checks

- canonical reference/architecture、shared DTO、producer、reader、Router、Runtime、tooling、test-runner与生态
  authoring使用同一snapshot/service DB语义；
- 反向搜索旧config literal/SecretRef/state binding/profile policy没有production、fixture、golden、sample或
  active task残留；
- config值不出现在四类artifact JSON、identity preimage、receipt、control frame、health或日志；
- activation exact-pair recovery、snapshot target environment在ConfigView物化前的strict比较、Package
  ConfigView隔离，以及storage-domain/environment/service-derived DB均有真实动态证据；
- `collection_name_mapping`在production authoring/schema/compiler/artifact/runtime/fixture中为零；logical
  collection identity只由provider DB declaration拥有，physical name由系统编码；
- runtime frame当前代际只有v3，旧v2 reader/writer/fixture均无兼容路径；
- service provider/callback只通过generation-pinned atomic rebinder切换owner；deployment-scoped
  config/DB/file/actor/spawn/WebSocket/telemetry完整重绑，request-scoped deadline/内部停止/time/request
  generation/lifecycle/trace/error/request identity/stream/test/heap limits保持不变；
- provider使用fresh heap和boundary materialization，caller actor frame不进入provider，ActorRef显式owner
  不改写，Package静态资源随RuntimeExecutionProjection，escaping stream保持旧generation pin；
- exact activation context缺失/歧义时fail closed，不读取latest、ambient或thread-local current service；
- 普通/unique DB index在prepare/cold recovery和prepared ACK前按同service全部version完整plan协调；
  missing additive create、exact pass、managed changed/removed fail closed、unmanaged与`_id_`保留，
  multi-replica exact create幂等且不发布partial prepared context；
- partial index由compiler拒绝，artifact/runtime无raw `where` AST；unique duplicate使用脱敏不可重试
  `std.db.ConstraintError`，不泄漏Mongo/database/collection/index/key/value；
- filtered migration只保留Agent定义/设置和credential/key/upstream，聊天/session/run/tool/interaction不
  进入目标；v1 secret重写为v2，旧库及迁移前备份仍保留；
- POSIX secret source未提交、是普通非symlink文件、mode精确`0600`且读取前检查；复制/暂存文件在使用前
  已是`0600`，snapshot store目录/文件为`0700`/`0600`，验收输出不泄漏内容；
- 无POSIX mode平台要么有明确可验证的owner-only/link-substitution等价边界，要么fail closed；
- full non-live、stable cold activation和Agine chat smoke证据属于同一最终候选。

第一行输出`PASS`或`FAIL`。FAIL必须指出唯一owner、代码证据、失效gate与最小修复边界；不得在验收worktree
直接修复。
