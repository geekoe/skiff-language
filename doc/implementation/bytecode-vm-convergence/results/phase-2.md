# Phase 2 Result

> Status: accepted by independent Acceptance receipt `PASS`; result landed on `main`
>
> Accepted candidate: `d0b0b69478b686220f1437b77808dd2238fdc077`
>
> Accepted tree: `5d5698e9ae1db7f9792e5993bd8275c99ace4677`
>
> Main integration merge: `41ee6355593357f5e4003d7c0664754dd8f723ef` / tree
> `6250d45950292828c7c6d7ece583fe1c336e0d1f`
>
> Acceptance verdict: `PASS`; waivers limited to the R0 R-FMT baseline red

## 1. Baseline and contract

Baseline 是已接受的 Phase 1 result main tip `e57e493f`。Phase Contract：
[`phase-2-value-lifecycle.md`](../phases/phase-2-value-lifecycle.md)（含 §3.4a/3.4b Amendment 1/2）；执行记录：
[`phase-2-execution-map.md`](../tasks/phase-2-execution-map.md)（MAP2，final revision recorded at acceptance）。

## 2. Delivered semantic closure

- emitter 的 plan 权威收敛到 source exact plan：全部形状推导与 `SnapshotRelease` fallback 已删除，缺失 plan 在
  emission 前稳定拒绝且不发布 artifact；`generated_slot_plan` 残留仅可达已禁用构式（后续 Phase 义务）。
- 唯一 linked-plan lifecycle executor（`runtime/vm/src/lifecycle.rs`）消费所有 slot 转换与 frame-exit；
  `reconcile_frame_slots_at` 已删除；`Trivial`（immediate 标量）计划走无 heap 交互快路径，Phase 1 的
  sidecar-free 证据保持成立。
- 两阶段 writable path（`prepare_writable_path` → RHS → `commit_writable_path` → slot = replacementRoot）：
  opaque non-Clone preparation、原子 commit 返回 replacement root、owner>1 时真实 COW、shared push fail closed；
  array-index 选择器权威统一为 `number`（integer-or-number 接受）。
- record/array（递归 number/bool/null 与嵌套聚合）在 compiler+linker 双边界 admitted；map/string/bytes/
  representation/stream/host/tail/throw/generic/`InOut` 全部 fail closed。
- VCP-2 经真实 authoring→route→production driver 证明 alias isolation（`a.inner.x`/`a.inner.tags` 不变、
  `b.inner.x==2`、`b.inner.tags==[9,2]`）与精确 share/COW/drop 序列（spy heap 经
  `heap: Option<Box<dyn VmHeap + Send>>` 注入并委托真实 heap）。

## 3. Evidence

- canonical Gate：`33/33` commands、`185/185` tests、candidate exact+clean、`checkerError: null`；
  evidence root `/Users/geek/workspace/skiff-bcvm-p2-acceptance-evidence/gate`，manifest SHA-256
  `37e397b9453de8952ad9bd86806689bc4263b3316156d6112d0f1e699e6d6ff8`。
- acceptance receipt [`phase-2-acceptance-receipt.md`](./phase-2-acceptance-receipt.md)，source SHA-256
  `cae742b4ae65851f783642a1fb87baef9befad9c7c30ce511854b501e88edbdd`；§7 checklist 全 `[x]`。
- 独立 review REV2 PASS（9/9）；preflight `/Users/geek/workspace/skiff-bcvm-p2-preflight-evidence-r2`。
- Phase 1 12 条回归与 11 事件观察/budget/terminal/cleanup 语义不变，全绿。

## 4. Disabled ledger and Phase 3 handoff

仍 disabled 且 fail closed：map/string/bytes/representation、ResourceRef/stream/host effect、tail call、throw/catch/
unwind 语言语义、generic/`InOut`、task/service/Actor/interface/callback、request GC、cross-owner heap。残留义务：
`generated_slot_plan`（for-in/match 重开前删除）、死代码 map-put/representation-wrap handler、COW 中途 OOM
孤儿克隆链论证。

Phase 3（outcome and unwind）从本 accepted main receipt 建 MAP3：统一 opaque exception envelope、真实 catch
identity、return/throw/VM failure/platform terminal、region cleanup 与跨 resume rethrow；lifecycle executor 的
frame-exit drop 协议已为 Phase 3 铺好。Phase 3 production 在本 result 合入 `main` 前不解禁。
