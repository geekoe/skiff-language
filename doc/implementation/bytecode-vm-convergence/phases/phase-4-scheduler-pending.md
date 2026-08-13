# Phase 4：scheduler, Pending and request ownership

> Status: active; activation authorized by the accepted Phase 3 result
>
> Semantic Closure: actual Pending、park/wake/claim、resume 恢复原 site、cancel/deadline/disconnect terminal
>
> Depends on: [`phase-3.md`](../results/phase-3.md) accepted and merged into `main`
>
> Unblocks: Phase 5 typed host effects, resources and streams

本文收敛 review findings VM-04/12/13。VM-13 的 typed key/revocation 已在 Phase 1 落地，本 Phase 只补
"session disconnect 终止其全部 request-owned Pending"；VM-04/12 是本 Phase 主体。

## 1. 目标

1. 唯一一个钉住的 host effect（canonical `std.time.sleep`）经单条权威链进入执行：artifact 只带 canonical
   binding ID → linker 从 pinned registry 构造 typed linked entry（不复制 artifact 自报签名）→ typed
   executor slot；删除 emitter effect rewrite、linker std bypass、request 字符串 dispatch；
2. 该 effect 的执行只返回 `Ready(result) | Pending(operation)`；真实等待由 Pending registry 持有，completion
   只经 terminal cell 唤醒 scheduler；**不同步等待后伪报 Ready**；
3. park 后 publish/wake/claim 各恰一次；resume 恢复原 VM site 恰一次；duplicate wake drop；
4. cancel/deadline/完成 竞争只产生一个 terminal；cancel-before-complete 与 deadline race 语义正确；
5. session disconnect 终止该 session 全部 request-owned Pending/fibers（VM-13 补齐）；
6. `PendingOwner<S: VmRootSource>` 的 root walk 组合 suspended invocation chain + transferred escrow +
   completion/wake values（stream buffer/resource/provider roots 属 Phase 5）；
7. 首个 production seam 用 **deterministic controlled completion**（fake host completion 注入 production
   边界，无真实 HTTP/时钟）。

## 2. 非目标

Phase 4 不：接真实 HTTP/时钟/stream/resource；实现 request GC/compaction（root graph 闭合是 GC 的前置，
不是 GC 本身）；扩大 host effect 面到 sleep 之外；恢复 adapter singleton 或字符串 dispatch。

## 3. 精确支持面

accepted：`std.time.sleep`（唯一 host effect）经 production compiler→linker→scheduler→request 链返回 actual
Pending，被 controlled completion 唤醒后恢复原 site 并返回确定性结果；cancel/deadline/disconnect 三条
terminal 路径。

仍 disabled 且 fail closed：其它全部 host effect、真实 HTTP/stream/resource、`Pending` 端口之外的伪 Ready。

## 4. VCP-4

真实 `.skiff` fixture 调用 `std.time.sleep`：经 production seam 返回 actual Pending（非伪 Ready）；fake host
completion 注入 production 边界完成之 → resume 恢复原 site、响应正确；证明一次 publish/wake/claim、owner
transfer 完整。negatives：cancel-before-complete → 单 terminal；deadline 竞态 → 单 terminal；duplicate wake
drop；session disconnect → 该 session 的 Pending 全终止、terminal 恰一次。stage-sentinel 矩阵覆盖每层门。

## 5. Acceptance checklist

- [ ] sleep 的 authority 单链（binding ID→typed entry→executor slot），四类第二权威删除；
- [ ] effect 只返回 Ready|Pending；无同步等待伪 Ready；
- [ ] publish/wake/claim 各一次、resume 一次、duplicate wake drop；
- [ ] cancel/deadline/disconnect 三条 terminal 路径各单次且互不重结算；
- [ ] root walk 组合 suspended chain + escrow + wake values；
- [ ] VCP-4 全绿；支持面外 host effect fail closed；
- [ ] Phase 1/2/3 回归 + fmt/clippy 自检全绿；
- [ ] frozen candidate 由全新 Acceptance Agent 给出 PASS；result 合入 main 后 Phase 5 解禁。
