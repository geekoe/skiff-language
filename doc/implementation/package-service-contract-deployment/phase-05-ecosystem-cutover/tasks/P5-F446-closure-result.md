# P5-F446 Closure Result

状态：**COMPLETE / R446 PASS**

本文记录F446实现收口；独立terminal结论仍由
[`P5-R446-unified-config-service-db-acceptance.md`](P5-R446-unified-config-service-db-acceptance.md)
给出，当前结论为`PASS`。

## Authority

- 父任务：
  [`P5-F446-unified-config-service-db-hard-cut.md`](P5-F446-unified-config-service-db-hard-cut.md)
- 直接实现任务：
  [`P5-F446A-artifact-compiler-state-removal.md`](P5-F446A-artifact-compiler-state-removal.md)、
  [`P5-F446B-config-snapshot-tooling.md`](P5-F446B-config-snapshot-tooling.md)、
  [`P5-F446C-activation-runtime-service-db.md`](P5-F446C-activation-runtime-service-db.md)、
  [`P5-F446D-test-runner-ecosystem-migration.md`](P5-F446D-test-runner-ecosystem-migration.md)
- 新发现的activation owner closure：
  [`P5-F448-activation-owner-switch-atomic-rebind.md`](P5-F448-activation-owner-switch-atomic-rebind.md)
- 新发现的service DB index closure：
  [`P5-F449-service-db-index-admission-and-migration.md`](P5-F449-service-db-index-admission-and-migration.md)
- 当前参考：
  [`config.md`](../../../../reference/config.md)、
  [`db.md`](../../../../reference/db.md)、
  [`runtime.md`](../../../../reference/runtime.md)

## Historical Integrated Checkpoints

2026-07-30中间复核基线是Skiff main `3344a5350a2ab593567ea38cbf5add4e9b8a1b8e`。该基线已经包含：

- `b54495e2`：从共享artifact schema删除deployment config/state binding；
- `29737432`：durable RuntimeConfigSnapshot protocol；
- `a61d8345`、`f79f2c77`：compiler config/state及dead binding projection hard cut；
- `5d9f5869`、`31664b46`、`d12f6838`：service policy及dead resource/runtime-capability binding删除；
- `6709393d`、`a803e289`：config snapshot projection与dev activation；
- `f8efb534`、`cbf98e42`、`1f9f65fe`、`73443258`：Router snapshot pair与retired binding删除；
- `dadd9fe9`、`575e37da`、`9f898155`：Runtime service DB、exact snapshot消费与dead binding删除；
- `25635b98`：test-runner execution snapshot隔离；
- `3344a535`：当前统一配置cutover integration convergence。

前置checkpoint报告曾记录以下聚焦证据：

- shared artifact checkpoint：artifact-model `181/181`、artifact-identity `102/102`及checker/fmt/diff通过；
- durable snapshot protocol checkpoint：artifact-model `184`、snapshot store `6`、transport `96`、
  artifact-identity `102`及clippy/fmt/diff通过。

这些证据证明相应checkpoint在当时的候选上成立；它们不覆盖下述新发现的收口遗漏，也不替代当前最终候选的
combined gate、跨仓库证据或R446。

## Required Closure Before R446

1. **Collection identity**
   - 删除`PackageDependency`、`PackageRequirement`、`PackageBinding`及package authoring中的
     `collection_name_mapping`全链；
   - `db object name`只表示declared logical collection identity；
   - physical collection稳定系统编码
     `(packageId, declared logical collection identity)`；
   - diamond same-build只保留一个metadata owner；不同Package同名保持隔离；test-only foreign target仍落
     caller generated test service DB。
2. **Snapshot environment**
   - `RuntimeConfigSnapshot`顶层required trusted `targetEnvironment`；
   - tooling、dev/watch、test-runner及其它producer显式写入；
   - Runtime prepare与cold recovery在物化任何`ConfigView`前严格比较activation environment。
3. **Service DB identity**
   - 统一为operator选择的受信Mongo endpoint/storage domain、environment与serviceId共同定界；
   - 不引入`platformId`，也不允许service/package authoring选择database name。
4. **Runtime wire**
   - Router↔Runtime frame统一为`skiff-runtime-frame-v3`；
   - producer、reader、strict validator、fixture与文档同代更新，不保留v2兼容reader。
5. **Managed dev watch**
   - 按[`P5-F447-managed-dev-watch-convergence.md`](P5-F447-managed-dev-watch-convergence.md)完成registry
     v2、dynamic input、last-known-good、bounded retry与health-derived CAS；
   - 配置只走RuntimeConfigSnapshot，不恢复旧YAML复制；F447聚焦验收不执行stable rollout。
6. **Activation owner switch**
   - exact `ActiveAssemblyContextSet`同时保留active及有pin的draining generation；
   - service provider与callback只通过generation-pinned
     `ActivationExecutionContextRebinder`原子切换owner，missing时fail closed且无latest fallback；
   - config、DB、file、actor、spawn、WebSocket、telemetry等deployment-scoped owner完整重绑；
   - deadline、内部停止、time、request generation/lifecycle、trace/error/request identity、stream、
     test effect/case capability与heap limits保持request-scoped；
   - provider fresh heap/boundary materialization、static resource projection、ActorRef显式owner、
     caller actor frame隔离及escaping stream旧generation pin均有动态证据。
7. **Service DB indexes and migration**
   - 普通/unique index在prepare与cold recovery、prepared ACK前按同service全部version的完整plan协调；
   - missing additive create、exact pass、managed changed/removed fail closed、unmanaged与`_id_`保留，
     multi-replica exact create幂等；
   - partial index compiler拒绝，artifact/runtime raw `where` AST全链删除；
   - unique duplicate只产生脱敏不可重试`std.db.ConstraintError`分类；
   - 本机旧库只迁移Agent定义/设置和credential/key/upstream；聊天/session/run/tool/interaction不迁移，
     v1 encrypted field重写为v2，旧库和备份保留。
8. **Final evidence**
   - 在上述实现合流后的同一精确候选上运行受影响聚焦测试、必要combined gate、跨仓库non-live、
     stable cold activation与Agine chat smoke；
   - R447先验收managed watch，R448独立验收activation owner switch，R449独立验收index/migration，再交由
     全新只读R446 owner执行terminal验收。

上述实现缺口已经完成、合流并分别通过R447、R448与R449。

## Terminal Closure

最终本机三仓状态：

- Skiff `6dfb38efce31d405390f238beed888381c5e3991` / tree
  `060162d25176eba73dc60471f5dcaf3f1cea71f0`；
- Internals `1e747618f26aa850d9fa10cba871e748d8e10609` / tree
  `33b60d604b7011e0b16ea63238afef8fabee87f6`；
- skiff-packages `db4ddd9e05936b6fa8beff42ed242c8a73f08de3` / tree
  `35f3102613bf0988c1c19adc53bfe1a7e2ef3b0f`。

三个仓库都在`main`、clean、未push；各自只有主worktree，没有已合并临时branch。F446 exact combined
checkpoint、F447 watch、F448 owner rebind与F449 index/migration的独立矩阵均PASS。

stable当前generation 12，assembly
`skiff-runtime-assembly-v3:sha256:bdeb7bfd7f8cebd5ef315308a150bba07e1b3fabaad071e63b2de7958a4beb3a`
与snapshot `skiff-runtime-config-snapshot-v1:5692b19db3d243c6bc6a81f0b004ade2`精确配对；
Router/Runtime healthy，pending activation为null，in-flight为0。active closure的171个File IR全部为v11。
Mongo `rs0`是writable primary，94个受管secondary index与14个unique index均存在；迁移检查点376条保留
记录的v2加密与ToolProvider清理收据通过，旧库与备份保留。

Agine用`aihub/deepseek-v4-flash`完成真实chat smoke及双Host验收。历史Codex OAuth refresh token被上游
`refresh_unauthorized`/HTTP 401拒绝是独立账号状态，不影响DeepSeek替代验收结论。

## Closed Secret Source Decision

secret source permission决策已经关闭：

- POSIX source必须是普通非symlink文件且mode精确为`0600`，读取内容前检查并fail closed；
- tooling任何必要明文复制/暂存写完后，必须先chmod到`0600`并重新确认，再允许使用或publish；
- 无POSIX mode平台必须明确并验证等价owner-only ACL、普通文件及link/reparse substitution防护；没有等价
  实现时fail closed；
- snapshot store目录/文件仍为`0700`/`0600`。

该规则已进入`config.md`、F446B/D与R446验收矩阵，不再是开放决策。

## Documentation Evidence

本文档任务只修改Markdown，不编译代码。完成前只执行：

- Markdown直接引用存在性检查；
- runtime frame、database identity、snapshot environment与collection mapping关键词反向搜索；
- `git diff --check`。
