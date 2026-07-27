# P5-F441O Encrypted live harness responsibility split

状态：Ready。F441K的behavior-preserving结构收尾；不运行live workload。

## 直接父节点

- `P5-F441N-encrypted-live-harness-responsibility-audit-result.md`
- `P5-F441K-encrypted-storage-live-harness-canonicalization-result.md`

F441N已证明三个稳定、单向依赖的owner；F441K冻结current command、receipt、activation、generation、
request与cleanup行为。本leaf只移动职责、增加hermetic seam/test，不改变语义或public API。

实现基线为`efee31bce177f865077340dad5aca4a2ec856282`。

## 目标

把`encrypted-storage-live-harness.mjs`收敛为facade/orchestration，并新建：

1. `encrypted-storage-live-contract.mjs`：纯command/receipt/request/lifecycle contract；
2. `encrypted-storage-live-mongo-probe.mjs`：mongosh/EJSON/storage discovery；
3. `encrypted-storage-live-instance-resources.mjs`：port/temp config/owned process group。

依赖只能从harness指向三个模块；三个模块不得import harness或互相形成循环。原harness路径继续导出所有
F441K public symbol，所有class方法与公开状态保持不变。

本节点是低风险结构检查点，不新增live能力，也不替代最终live gate。

## 唯一写集

- `scripts/lib/encrypted-storage-live-harness.mjs`
- 新建：
  - `scripts/lib/encrypted-storage-live-contract.mjs`
  - `scripts/lib/encrypted-storage-live-mongo-probe.mjs`
  - `scripts/lib/encrypted-storage-live-instance-resources.mjs`
- `scripts/tests/encrypted-storage-live-harness.test.mjs`
- 可新建：
  - `scripts/tests/encrypted-storage-live-mongo-probe.test.mjs`
  - `scripts/tests/encrypted-storage-live-instance-resources.test.mjs`
- 本leaf result

禁止修改checker调用方、source roots、test-runner、verify plan、Router/Runtime、live config、其它
fixture/task/result。不得派子Agent，不得启动Mongo/instance/server或网络。

## Owner边界

### Contract模块

移动并保持精确行为：

- `repoRoot`与contract constants；
- `encryptedStorageTestRunnerArgs`；
- `encryptedStorageBuildArgs`；
- `encryptedStorageProductionAssembly`；
- `encryptedStorageIngressRequest`；
- `runEncryptedStorageTestLifecycle`；
- 上述函数独占的pure validator/helper。

harness从原路径re-export原有public contract symbol。除已有export外不扩张public surface；内部常量只在必要
时模块内export。

### Mongo probe模块

移动mongosh command、`mongoJson`及数据库/collection/document方法、transient storage observation、
replica-set initialization、envelope/database decoder。probe通过constructor/factory只接收必要的
`mongoPort`、`cwd`和测试可注入command/delay seam，不读取harness。

harness保留原同名方法并delegation；checker不能感知模块拆分。

### Instance resource模块

移动port range/forbidden rules、lease、instance paths/config factory、owned process-group
validation/termination及其helper。factory只返回resources，不构造harness。

必须保持cleanup的公开顺序和状态：

1. 校验owned process group；
2. harness记录`cleanupFallbackUsed`/`cleanupFallbackGroups`；
3. TERM；
4. bounded wait；
5. 只对survivor KILL。

测试注入不能进入production public API，也不能削弱process ownership检查。

### Harness facade

保留initialize/build/test/activation、keyring/restart、HTTP retry、cleanup orchestration、command/log
orchestration及Mongo/resource delegation。不得顺手拆分其它helper或合并不同instance implementation。

## 测试先行与验证

先增加export/owner或dependency assertion，使未拆分代码至少一项失败，再移动。必须覆盖：

- 原harness import path的export names精确不变；
- F441K所有runner/build/receipt/request/lifecycle断言不变；
- retired-surface扫描三个新模块；
- Mongo fake command验证URL、expression、cwd与读写/canned decode；
- instance fake seam验证port禁区、config文本、错误ownership拒绝、TERM/wait/survivor KILL顺序；
- dependency scan证明三个新模块不import harness且无循环。

必跑：

```bash
node --test \
  scripts/tests/encrypted-storage-live-harness.test.mjs \
  scripts/tests/encrypted-storage-live-mongo-probe.test.mjs \
  scripts/tests/encrypted-storage-live-instance-resources.test.mjs
node --check scripts/lib/encrypted-storage-live-harness.mjs
node --check scripts/lib/encrypted-storage-live-contract.mjs
node --check scripts/lib/encrypted-storage-live-mongo-probe.mjs
node --check scripts/lib/encrypted-storage-live-instance-resources.mjs
git diff --check
```

未新增某个可选test文件时删除不存在路径并在result记录实际文件/count，不得用零测试充当证据。

## 停止与交付

若保持public API/行为需要循环依赖、修改checker或改变activation/cleanup语义，返回
`TASK_SCOPE_EXPANDED`并保持原实现；不得为完成拆分改行为。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f441o-encrypted-harness-split`
- branch：`codex/p5-f441o-encrypted-harness-split`
- result：`P5-F441O-encrypted-live-harness-responsibility-split-result.md`

Implementation与result分开提交；不merge/rebase/push。
