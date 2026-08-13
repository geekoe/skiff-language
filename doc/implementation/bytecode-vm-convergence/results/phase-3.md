# Phase 3 Result

> Status: accepted by independent Acceptance receipt `PASS` (round 3); result landed on `main`
>
> Accepted candidate: `411da4559ec2e61df525e5dd8baf68fac5175e45`
>
> Accepted tree: `a8c225df65d471f50cf13eb18bd01551168b216b`
>
> Main integration merge: `<recorded after merge>` / tree `<recorded after merge>`
>
> Acceptance verdict: `PASS`; waivers limited to the R0 R-FMT baseline red

## 1. Baseline and contract

Baseline 是已接受的 Phase 2 result main tip `0fee73cd`。Phase Contract：
[`phase-3-outcome-unwind.md`](../phases/phase-3-outcome-unwind.md)（含 §4a/4b Amendment 1/2）；执行记录：
[`phase-3-execution-map.md`](../tasks/phase-3-execution-map.md)（MAP3，final revision recorded at acceptance）。

## 2. Delivered semantic closure

- `RequestException` 是唯一 opaque exception envelope：`throw` 从被抛值运行时叶 tag 构造 actual concrete leaf
  identity；`rethrow` 经 `Exception<E>.error` payload handle 复用同一 envelope（identity 不变）；`resume_throw`
  经两阶段 unwind 消费同一 opaque envelope（live 测试 `Arc::ptr_eq` + 五要素 identity）。
- 根级未捕获 throw 以 typed outcome 传出（`VmError::UnhandledThrow` 已删）；scheduler 不压 terminal failure；
  request 投影 Throw→`std.service.InternalError`/"uncaught user exception"、envelope VmFailure→sanitized
  `InternalError`、PlatformTerminal 不变；terminal 恰一次。
- catch 按 actual `CatchIdentity` 匹配（union 叶 A 匹配 `catch<A>` 不匹配 `catch<B>`）；unwind 每层 frame-exit
  走 Phase 2 lifecycle executor，cleanup owner 释放由 heap spy 证明。
- 窄支持面（§4a/4b）：string literal 仅作 discriminator 常量（`Builtin("string")` 通用值 fail closed）；
  异常 payload 只放行 nominal record / union nominal 分支，scalar/structural/literal 叶在 emission admission
  稳定拒绝；窄 union 分支可赋值（叶→含叶匿名 union + plan 相等）只限槽写/call 参数。

## 3. Evidence

- canonical Gate：`46/46` commands、`272/272` tests、candidate exact+clean、`checkerError: null`；
  evidence root `/Users/geek/workspace/skiff-bcvm-p3-acceptance-evidence-r3/gate`，manifest SHA-256
  `2c8d1ff67e765a114424edafa34bb98e5e242426487ed098f364853814d9559e`。
- acceptance receipt [`phase-3-acceptance-receipt.md`](./phase-3-acceptance-receipt.md)，source SHA-256
  `add410eaf0615c5ab85a1d2f90e62efc64c81d3da0069f78d67dcb4d418d5ab6`；§7 checklist 全 `[x]`（时序项除外）。
- 独立 review REV3 PASS（含 delta re-check）；fmt 逐文件归因：Phase 3 写入行 0 rustfmt 残留（550 处全为旧漂移）。
- Phase 1（11 事件/budget/terminal/cleanup）与 Phase 2（lifecycle/COW/alias）回归全绿。

## 4. Disabled ledger and Phase 4 handoff

仍 disabled 且 fail closed：host effect、Pending/child/stream 相关 throw（live 面）、service error 恢复语义、
platform error envelope、scalar/structural/literal 叶 throw、通用 string 值、tail call。后续义务：literal-branch
identity（enclosing type id + payload literal）、根级未捕获 envelope payload 的 request-heap teardown 语义、
map-put/representation-wrap 死代码 handler。

Phase 4（scheduler, Pending and request ownership）从本 accepted main receipt 建 MAP4：Ready/Pending、
park/wake/claim、cancel/deadline race、suspended invocation roots、session-owned request、child control frame；
先用 deterministic controlled completion，不同时接真实 HTTP。Phase 4 production 在本 result 合入 `main` 前不解禁。
