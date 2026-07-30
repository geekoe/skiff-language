# P5-D93 Suspension current-base reconciliation audit

状态：Ready（只读审计；P5-F415 合流后才能启动实现 DAG）。

## 直接父节点

- `P5-F395-inferred-suspension-implementation-audit-result.md`
- `P5-F409-typed-service-selection-contract-driver-result.md`
- `P5-F413-relay-service-calls-and-http-checkpoint-migration-result.md`
- `P5-F415-collection-mapping-current-integration.md`

F395 已冻结 inferred suspension 的语言与 runtime 语义及 A–E / N0–N5 DAG。此后 serviceCalls
切代占用了 PackageArtifact v8/build v9，并把 ServiceDeploymentInput 升为 v3；Relay current
authoring 也已能进入 identity projection，真实暴露 interface requirement `maySuspend=false` 与
concrete implementation `maySuspend=true` 的错误比较。F415 正在把 collection mapping safe
checkpoint 适配到 current integration。

本审计不重新讨论语言设计，只把 F395 的代码事实、generation 与 DAG 精确重放到最新代码状态，供
F415 后的实现任务直接使用。

## 审计锚点

- Skiff current audit start：
  `91e5475d18af9b30adcc01dc4ea2ba41e3d1e10b`。
- Internals current integration：
  `960cc4bd722cbbad41fdd5e064663ad505e4f3ac`。
- F415 candidate：
  `/Users/geek/workspace/skiff-p5-f415-collection-mapping-current`（只读观察；不得修改或依赖未提交临时
  hunk作为事实）。

最终 implementation start 必须是包含 F415 accepted commit 的 Skiff integration descendant；审计
result 要明确写出这一 gate。

## 只读范围与必须回答

可读取 Skiff、F415 candidate、Internals integration；只允许新增本任务 result，不改
production/test/design。

### 1. Current 字段与 owner

按 F395 A–E 重新枚举：

- requirement-owned `InterfaceMethodSignature.may_suspend`；
- callback requirement `BoundaryCallbackOperation.may_suspend`；
- ServiceContract-owned `BoundaryOperationContract.may_suspend` 与 `cancellation`；
- 必须保留的 concrete executable / Package callable / semantic facts /
  `completeMayEffects`；
- compiler call-target fixed-point；
- deployment eligibility；
- runtime ordinary/async lane、deadline/cancellation owner；
- Router/scripts/cross-system direct consumers。

给出 exact production owner、test/fixture family、数量与反向搜索。禁止把同名 concrete facts误列为
删除项。

### 2. Current generation 表

从当前 constants/schema/strict reader 读取实际值，给出唯一不复用 generation 的终态。至少明确：

- serviceCalls 已占用 PackageArtifact v8、canonical build v9 与 build preimage marker；
- interface/callback grammar 再变化时 PackageArtifact、canonical Local ABI、canonical build、
  PackageSchemaType、PackageUnit、legacy implementation-links/build 各自应切到什么新 generation；
- ServiceContract/protocol 从 current v4 切到什么；
- ServiceDeploymentInput 保持 current v3，ServiceDeployment 与 RuntimeAssembly 保持 v2；
- collection mapping 字段必须保留，不能在 suspension fixture adaptation 中丢失。

每项列出 marker/prefix/schema、current→terminal、变化原因与 strict negative。

### 3. G0 与真实 blocker

用 current Skiff + Internals receipt/error证据证明：

- current Relay root 已由 canonical package/service authoring读取；
- serviceCalls 精确选中 `relayProxy`；
- authoring 已越过旧 manifest/parser，终止于 suspension requirement/concrete mismatch；
- 因此 F395 的旧“service-only source-base skew” G0 已关闭，不需要恢复旧 package/API marker。

不得通过临时改 Relay source、waiver 或 stable store制造通过。

### 4. 可执行 DAG

把 N0–N5 更新为 current owner/write set，保持：

```text
post-F415
  -> N0 schema/identity
  -> N1 compiler || N2 deployment || N3 runtime
  -> N4 Router/tooling/current-generation oracles
  -> N5 fresh Relay-first ecosystem proof
```

说明哪些 task 文件可由主 Agent直接派生、每个节点 exact allowed write set、focused tests、negative
matrix、合流顺序和证据失效条件。特别纳入当前完整源码套件暴露的
`scripts/lib/skiff-source-test-suite.mjs` v1 validator，以及其它 production script v1/v4
positive oracles；不要机械替换作为 legacy rejection 的字符串。

若 current code 暴露会改变 F395 语义的真正冲突，返回需要用户决定的最小问题；否则必须给出
`TASK_EXECUTABLE`，不得把代码量大当作设计 blocker。

## 验证与交付

运行只读搜索、必要的 `cargo test -- --list` / `pnpm vitest list` 与 current Relay authoring
diagnostic；不得运行 stable/live、外部服务或修改 F415 worktree。不得派子 Agent。

写 `P5-D93-suspension-current-base-reconciliation-audit-result.md`，记录 exact commit/tree、完整
generation 表、owner/DAG、Relay诊断与任务合同输入。只提交 result，worktree clean；不
merge/rebase/push。
