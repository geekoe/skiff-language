PASS

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

## Result Record

terminal审计锚定：

- Skiff `6dfb38efce31d405390f238beed888381c5e3991` / tree
  `060162d25176eba73dc60471f5dcaf3f1cea71f0`；
- Internals `1e747618f26aa850d9fa10cba871e748d8e10609` / tree
  `33b60d604b7011e0b16ea63238afef8fabee87f6`；
- skiff-packages `db4ddd9e05936b6fa8beff42ed242c8a73f08de3` / tree
  `35f3102613bf0988c1c19adc53bfe1a7e2ef3b0f`。

三仓均为clean `main`且未push。昂贵验证由各exact实现owner执行，没有在terminal审计中机械重跑：

- F446 combined checkpoint覆盖artifact、compiler、snapshot、activation、service DB、test-runner、Host、
  Router与Node链路；
- R447 watch/registry、R448 activation owner rebind、R449 index/migration分别PASS；
- F449最终candidate的foundation、compiler、runtime、rust-quality、checks及test-runner完整selector PASS；
- stable generation 12 active pair healthy，pending为null，单Runtime replica connected/healthy且in-flight为0；
- 171个active File IR全部v11，94个Mongo managed secondary index与14个unique index存在；
- 迁移检查点376条记录，5个credential record的所有目标加密字段均为v2；旧库、archive及旧artifact root保留；
- POSIX secret/snapshot/receipt权限与SHA manifest复核通过；
- Agine DeepSeek chat smoke exit 0，双Host Playwright `1 passed`且exit 0。

当前live数据库在验收后已有新运行记录，因此不要求总数仍等于迁移检查点376；索引、v2密文和保留数据不变量
仍成立。旧Codex OAuth refresh token的上游401属于账号状态，未被误写为runtime或迁移失败。

结论：Blocking Checks全部关闭，R446 **PASS**。
