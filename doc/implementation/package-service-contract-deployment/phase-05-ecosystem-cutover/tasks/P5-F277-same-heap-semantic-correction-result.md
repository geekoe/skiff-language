# P5-F277 Same-heap semantic correction result

状态：Design decision complete；解除 same-heap 实现节点。

## 直接父节点与权威链

- 直接父结果：
  `P5-F276-contextual-same-heap-audit-result.md`
- 诊断父结果：
  `P5-F275-aihub-same-heap-diagnostic-result.md`
- 唯一架构事实源：
  `doc/architecture/package-service-contract-deployment.md` 第 7、8 节

本结果记录用户确认后的设计修正。若本结果与权威架构冲突，以权威架构为准。

## 已确认语义

`requiresSameHeapIdentity` 只表示 callable 可能对 caller-reachable heap 引用执行了引用身份敏感
操作，例如 heap value 的 `==` / `!=`。它不是 alias、mutation、escape、interface boxing、
callback capability 或 unknown target 的总括位。

以下事实必须独立：

- collection/field read 和直接返回引用只产生 return provenance / `returnsCallerAlias`；
- caller 图 mutation 只产生 write provenance / `writesCallerReachable`；
- throw alias、escape、callback 和 unknown 分别使用已有 owner；
- unknown target 由 unknown/effect fact 失败关闭，不伪造已发生的 identity observation；
- fresh/local 引用之间的 identity 比较不依赖 caller heap，不上浮该位；
- 对 caller-reachable 引用的 identity 比较一旦发生，不得由 fresh consumer、detached materialization、
  DB escape 或其它后续操作消除。

任何 public callable 只要可能对 service boundary 输入执行引用身份比较，就不能成为 service
operation。普通 Package API 仍可使用这种算法。

## 对 F276 的修正

F276 建议为 PackageArtifact 增加结构化 `sameHeapIdentity` owner wire。该建议建立在
`Map.get`、mutation 和 identity observation 共用同一位的前提上，现已被本结果取代。

当前节点不增加新 wire：

- 保留 `CallableMayEffects.requiresSameHeapIdentity` 布尔值；
- 保留 compiler 内部 parameter attribution，用于同一 source/package 分析中的上下文化；
- 先删除 alias/mutation/boxing/unknown 对该位的错误写入；
- 只有将来真实 identity comparison 出现跨 Package 参数归属误报，才另行审议是否增加参数级
  artifact facts。

AIHub 的 `Map.get -> std.json.merge(Fresh)` 路径没有执行引用身份比较，因此不得再携带
`requiresSameHeapIdentity`。直接返回 `Map.get` 仍以 `returnsCallerAlias` 拒绝；caller mutation
仍以 `writesCallerReachable` 拒绝。

## 实现与验证边界

后续实现必须覆盖所有 production setter/consumer，而不是只修 AIHub 链：

- builtin receiver exact semantics；
- expression equality、interface boxing 和 unresolved target；
- field/collection store 与 callee parameter-store transfer；
- unknown/fail-closed state；
- boundary eligibility；
- source、artifact 和 test-runner 的对应测试。

禁止修改 AIHub/Agine 源码、增加函数名或 Package ID 特判、扩展 semantic-facts wire、放宽 alias /
mutation boundary gate，或改变 runtime equality。

