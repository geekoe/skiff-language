# MAP2：Phase 2 rolling execution map

> Status: active; revision 3; K2 lifecycle kernel delivered, contract amendment 1 recorded, awaiting C2 admission join
>
> Phase Contract: [`phase-2-value-lifecycle.md`](../phases/phase-2-value-lifecycle.md)
>
> Phase 1 input: [`phase-1.md`](../results/phase-1.md) accepted on `main`
>
> Baseline commit: `e57e493f94ace9a864a38b49c3f1ad1ab9171ea3`
>
> Integration branch/worktree: `codex/bcvm-p2-integration` / `/Users/geek/workspace/skiff-bcvm-p2-integration`

## 1. Activation receipt

Baseline 是已接受 Phase 1 result 的 `main` tip（`e57e493f`），其 merge `77ec6665` 含 accepted candidate
`18412953`/tree `73974228` 与两轮 Acceptance receipt。Phase 1 Gate 的 24 命令矩阵成为 Phase 2 的永久回归
selector。main checkout 保持在 `main`，clean，与 origin 同步。

## 2. Target and containment ledger

Phase 2 只把 `record`/`array`（递归 immediate scalar + 嵌套聚合）从 `disabled` 迁到 `accepted`。其余
`map`/`string`/`bytes`/representation/ResourceRef/stream/host effect/tail call/throw/generic/`InOut`/
task/service/Actor/interface/callback/request GC/cross-owner heap 全部保持 `disabled` 且 fail closed。
Phase 1 的 scalar/local-call/budget/observation 语义在本 Phase 不改变。

## 3. Initial ready frontier

所有 worktree 是 `/Users/geek/workspace` 的直接子目录；每个 writer 从 MAP2 commit 出发；agents 无父对话，
必须自行读 Phase Contract、Phase 1 result 与本 MAP。三条 lane 首日并行，写面不重叠。

| Lane | Role / Agent ID | Worktree | Exact write ownership | First checkpoint / expected handoff |
| --- | --- | --- | --- | --- |
| K2 | central kernel | `skiff-bcvm-p2-kernel` | `runtime/model/src/vm_heap.rs`（trait 两阶段协议 + `WritablePathPreparation`）、`runtime/vm/src/{fiber.rs,lib.rs,lifecycle.rs}`、`runtime/request/src/vm_heap.rs`、`runtime/request/src/bytecode_ingress.rs`（仅 heap seam 相关）、`runtime/linker/src/bytecode/link/capability.rs`（仅 record/array admission 放宽）及上述文件 inline tests | 10m / 40m |
| C2 | compiler plan lane | `skiff-bcvm-p2-compiler` | `compiler/source/src/value_transfer*.rs`（facts 管线导出）、`compiler/driver/pipeline/bytecode_lane.rs`、`compiler/emission/src/bytecode/plans.rs`、`compiler/emission/src/bytecode/{functions.rs}`（仅 plan 消费相关）及 inline admission tests | 8m / 30m |
| P2G | Proof + Gate lane | `skiff-bcvm-p2-proof-gate` | 新 `runtime/host/src/host/request_entry/phase_2_proof_support*.rs`、`phase_2_vcp_tests.rs`、`scripts/lib/bytecode-vm-phase-2-*.mjs`、`scripts/run-bytecode-vm-phase-2-gate.mjs`、`scripts/tests/bytecode-vm-phase-2-gate-*.test.mjs`、Gate selector 注册（`scripts/verify.mjs` 或 registry） | 8m / 30m |

Integrator 只做机械 cherry-pick、receipt/MAP 更新、Gate/freeze/Acceptance 编排；不在 merge 时补 plan、默认值、
第二 API 或生命周期语义。K2 是唯一 central kernel owner，不因 crate 边界拆给多个 owner。

## 4b. Revision 2

- P2G 首日交付 `1919af92` + `03aa7da7`：expected-red VCP-2 harness（真实 fixture 经 production seam，heap spy 已
  编译级实现）与 missing-plan negative；Phase 2 Gate 共 33 命令（12 probes + 21 workloads：9 条 Phase 2 场景 +
  12 条 Phase 1 全量回归），Node 自测 24/24 与回归 59/59 绿，VCP/negative 以真实断言红（exit 101，0 skip/ignore/todo）。
- heap 注入 seam 经证实横跨 request 与 host：`drive_runtime_bytecode_request` 需增加 `heap: Option<Box<dyn VmHeap +
  Send>>` 传递，且 `runtime/host/src/host/request_entry/assembly.rs` 是 host 侧透传点。该文件加入 K2 写界（仅最小
  透传，host 默认 None，proof 可注入；不新增公共执行器）。
- join 契约过滤词固定：K2 = `lifecycle`（vm lib）、`vm_heap`（model/request lib）、`capability`（linker lib）；
  C2 = `phase_2_bytecode_admission`（emission 与 compiler lib）。producer 测试必须按词精确命中且非零。
- VCP fixture 源语义修正：Skiff 别名突变需 `var b = a`（`final` 绑定不可作赋值目标）；fixture 已按此落地。

## 4c. Revision 3

- K2 交付 `cade87e7..b330dff9`：model 两阶段 writable path（opaque `WritablePathPreparation`、atomic commit 返回
  replacement root）、唯一 linked-plan lifecycle executor（`lifecycle.rs`，`reconcile_frame_slots_at` 已删）、
  request heap owner 记账 + commit COW + 递归 drop、`BytecodeRequestExecutionInput.heap` spy seam（host 默认 None）、
  linker capability record/array 放宽。focused 全绿（model 136/vm 56/request 52/linker 54），三包全量 298 过 0 红。
- 两处编译必要性最小改动记入写集：`websocket_jsonrpc.rs` 与 `runtime/request/tests/bytecode_request.rs` 各加
  `heap: None`（struct 字面量适配，无语义变化）。
- Phase Contract §3.4a Amendment 1 定案：单指令 `SetWritablePath` 下 RHS 由前序指令求值；Phase 2 纯值表面内
  prepare-before-commit + atomic commit 成立，“RHS 宿主副作用不提前发生”是 Phase 5 的发射形状前置；shared
  container 的 push 在 Phase 2 fail closed，不隐式 COW push。
- K2 join 契约过滤词：`lifecycle`（vm lib）、`vm_heap`（model/request lib）、`capability`（linker lib），均非零全绿。

## 4. Task contracts

### K2 — lifecycle kernel（VM-01 第 3/4/5 条 + VM-02）

- model trait：删除 `set_writable_path`，新增 `prepare_writable_path`/`commit_writable_path` + opaque
  `WritablePathPreparation`（non-Clone；VM 不检查内部）；
- 新 `runtime/vm/src/lifecycle.rs`：唯一 executor，消费 `frame.slot_plans()` 的 `LinkedValueTransferPlan`；
  所有 slot 转换与 frame-exit 路由到它；删除 `reconcile_frame_slots_at`；
- request heap：`snapshot_share`/`transfer_owner`/`release_snapshot` 精确 owner 记账；commit 路径按 owner count
  COW 并返回 replacement root；递归 snapshot/resource drop 协议；
- `execute_set_writable_path` 顺序固定 prepare → RHS → commit → install replacement root；中间 selector 失败时
  RHS 未求值；
- linker capability 只放行 record/array 的 construction/load/mutation/transfer；其余 aggregate 能力仍拒绝；
- 不改变 Phase 1 观察事件、budget、terminal、cleanup 语义；错误路径零 observation、heap state 不变、重试安全。

### C2 — exact plan pass-through（VM-01 第 1/2 条）

- pipeline 把 `SourceValueTransferFacts` 传入 emission；`derive_bytecode_value_transfer_plans` 消费
  `source_value_transfer_plan` exact 结果；
- 删除全部启发式与 `SnapshotRelease` fallback；缺失 plan 返回稳定 typed `BytecodeEmissionError` 且不发布 artifact；
- 不扩大 compiler admission 之外的支持面；record/array 的 source 构造仍走既有 lowering，仅 plan 权威切换。

### P2G — expected-red VCP + negative + Gate（首日含全部 scenario）

- VCP harness 用真实 fixture 经 production seam，注入 heap spy（`drive_runtime_bytecode_request` 的
  `heap: Option<Box<dyn VmHeap + Send>>`）证明 share/COW/drop 调用序列；首日 expected-red；
- missing-plan negative expected-red；Phase 1 24 命令回归全纳入 Phase 2 Gate；
- Phase 2 Gate selector + Node self-tests 首日即含 VCP/negative 场景（红由 harness 证明，不是 skip）；
- Gate 不重实现生命周期语义，只聚合证据；任何 producer 由红转绿时同 join 收进矩阵。

## 5. Integration and validation order

1. C2 先合（不依赖 K2），合后跑 compiler admission focused；K2 与 C2 的 producer-consumer join 一起跑 VCP；
2. cargo 共享 target `/Users/geek/workspace/.skiff-cargo-target`；跨 worker 用 `/tmp/skiff-p2-cargo-lease`
   目录租约串行（`mkdir` 抢租、`rmdir` 释放、`sleep 5` 轮询），任何 cargo 命令必须持租；
3. >30s 命令重定向 `/tmp` 并轮询，不复跑只为找回输出；
4. 每个 join 后跑受影响的最小 preflight；P2G 的 Node 自测可随时跑；
5. merged preflight 全绿后 freeze、新 detached Acceptance worktree、全新 Acceptance Agent 跑完整 Phase 2 Gate。

## 6. Watchdog and takeover

- checkpoint = 可见 code/test/decision 输出 + 当前 blocker；
- 首 checkpoint 按 §3 表；超时 15 分钟无 diff 要求部分提交或停止；30 分钟无可信 handoff 则打断并由新
  owner 从最后可信 commit 接管原 worktree；
- K2 永不拆 owner；read-only 诊断可并行，write authority 单一。

## 7. Candidate and evidence epochs

integration line 不是 acceptance 候选。所有 Development/Proof 绿后 integrator 跑 merged preflight、freeze exact
commit/tree、创建 detached Acceptance worktree。此后任何 production/test/fixture/Gate/schema 变化开新 epoch。
只有全新 Acceptance Agent 可出最终 verdict。
