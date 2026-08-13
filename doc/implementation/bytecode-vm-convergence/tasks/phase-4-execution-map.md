# MAP4：Phase 4 rolling execution map

> Status: active; revision 3; all four lanes joined, merged preflight pending; paused at the Duration-construction gap (no new agents per user)
>
> Phase Contract: [`phase-4-scheduler-pending.md`](../phases/phase-4-scheduler-pending.md)
>
> Baseline commit: 当前 main tip（Phase 3 accepted 后）
>
> Integration branch/worktree: `codex/bcvm-p4-integration` / `/Users/geek/workspace/skiff-bcvm-p4-integration`

## 1. Gate map（预调查结论）

1. C4 admission：只放行 `std.time.sleep`，删 emitter effect rewrite；
2. V4 linker：typed entry 只取 pinned registry，删 std bypass；
3. V4 verifier：证明 linked typed ID + 调用栈事实 + `ActualWithResume{HostEffect}` pending contract；
4. K4 VM：host-effect opcode 返回 actual Pending（resume token），不伪 Ready；
5. K4 scheduler：park 进 PendingRegistry、wake/claim、resume 恢复原 site、root walk 组合；
6. K4 request：driver 接真 ports（非 default-absent），Parked 不再 Unsupported，cancel/deadline race；
7. K4 host：session disconnect 终止该 session 全部 Pending/fiber；
8. P4G VCP：fake completion 注入 production 边界，正面+negative 全断言。

## 2. Lanes（写面分区，无固定模板）

| Lane | Worktree | 写集 | join 顺序 |
| --- | --- | --- | --- |
| K4 kernel | `skiff-bcvm-p4-kernel` | `runtime/scheduler/src/*`、`runtime/request/src/bytecode_ingress.rs`（+execution_budget 如需要）、`runtime/host/src/host/{request_supervisor,router_session}.rs`、`runtime/vm/src/{control,fiber}.rs`（Pending 返回/resume 路径） | 3 |
| V4 link/verify | `skiff-bcvm-p4-verify` | `runtime/linker/src/bytecode/{link/*,stack_map/*}`（typed host entry、删 bypass）、`runtime/bytecode-verifier/src/**`（pending contract 证明） | 2 |
| C4 compiler | `skiff-bcvm-p4-compiler` | `compiler/emission/src/bytecode/{admission,functions}.rs`、`compiler/driver/pipeline/bytecode_lane.rs` | 1 |
| P4G proof+gate | `skiff-bcvm-p4-proof-gate` | 新 phase_4 proof 文件、`scripts/lib/bytecode-vm-phase-4-*.mjs`、`scripts/run-bytecode-vm-phase-4-gate.mjs`、自测、verify 图注册 | 4 |

写集是唯一权威；扩展先上报，获准后先改本 MAP 再动代码。中央 Pending 状态机只归 K4。

## 3a. Revision 2

- K4 交付 `57ff2237`/`e8d8c138`/`ebf3ed55`：`PendingOwner<S: VmRootSource>` 根遍历组合（suspended chain +
  escrow + wake values）、`RequestExecutionContext` 多轮驱动 seam、`ExecutionBudget` 成为 pending 终局唯一权威
  （`RequestPendingSink`，settle 恰通知一次）、driver 接真 ports（`SleepHostExecutor`/`VmPendingRegistry`/wake
  queue，`Parked` 不再 Unsupported）、`drive_runtime_bytecode_request_controlled` 受控 seam、host stop_session/
  cancel 经 budget sink 终止 Pending 恰一次零 observation 泄漏。三包 317 全绿，Phase 1/2/3 回归全绿。
- 写集扩展记录：`runtime/request/src/lib.rs` 4 行 re-export（受控 seam 的导出点）纳入 K4 写集。
- 接口结论：sleep binding key `std.time.sleep`；`complete() -> bool`（false=duplicate drop）；P4G 用
  `drive_runtime_bytecode_request_controlled`；VM 无需 diff（`InvokeHost`/`EnterAdapter`/`resume_inner` 已就绪）。

## 3b. Revision 3

- C4 `e69607a2`/`85998881`：admission 单放行 `std.time.sleep`（精确 arity/type/void），删除 emitter effect
  rewrite；V4 `fe32126a`/`eee75d4a`：typed entry 只从 pinned registry 构造、删 std mismatch 吞掉路径、
  verifier 证明 `ActualWithResume{HostEffect}`（pending 类别归 registry 权威）；P4G `0caeeb67`/`1e320414`/
  `2f41cd09`：full-chain VCP + 6 stage sentinels + 4 negatives + Gate（67 命令基线已跑，11 P4 场景真实红、
  34 回归全绿，baseline `/private/tmp/skiff-p4-gate-baseline`）。
- 四 lane 已合入 integration（tip `65e3db1c`）。
- **已知 blocker（转绿前）**：真实 fixture 无法构造 `Duration`——`Duration.milliseconds(...)` 被 admission
  当作第二个 binding 拒绝，直接接收 `Duration` 参数又缺 std nominal transfer facts。归 C4（构造器常量折叠或
  纳入 sleep authority 链）。修复前 VCP 在 publish 边界红。
- **暂停点**：按用户指令，不再派发新 agent。剩余顺序：C4 修 Duration gap → 转绿 → Gate preflight →
  freeze → 独立 review → 全新 Acceptance → results/phase-4.md 合 main。这些留待恢复后继续。

## 3c. Duration gap 的精确修法（恢复时按此执行，归 C4 lane）

根因已定位（integrator 只读调查）：

1. `core.duration.milliseconds` 是 `Context::Time` 的**纯构造器**（非宿主副作用），`after(200ms)` 在 source
   层 desugar 成它；当前 emission admission 只放行 `std.time.sleep`，把该纯构造器当第二个 binding 拒绝；
2. `compiler/lowering/src/const_evaluator.rs` 没有 Duration 常量折叠；`std.time.Duration` 是 prelude 名义类型，
   `SourceValueTransferFacts` 无其 nominal facts，导致 transfer-plan 失败。

修法（三处，同属 compiler 写面）：

- admission：把 `core.duration.milliseconds` 放行为纯构造器（无 pending、无宿主效应），不并入 sleep authority；
- lowering：`Duration.milliseconds(<literal>)` 常量折叠为 Duration 常量（或可物化的 immediate 表示），不产生
  运行时宿主调用；
- value-transfer：为 `std.time.Duration` 注册 exact source plan（沿用 native lifecycle registry 路径），使
  sleep 的参数槽可链接。

完成后 VCP-4 的 11 个场景（full-chain + 6 哨兵 + 4 negative）应转绿；否则逐 gate 收敛。

## 3. 验证与纪律

- cargo 租约 `/tmp/skiff-p4-cargo-lease`（mkdir 抢租/rmdir 释放）；focused 每轮跑，三包/全量只在 join 点跑；
- Gate 矩阵首日含 P4 全部 scenario（expected-red）+ fmt/clippy 自检 + Phase 1/2/3 全量回归；
- 每扇门转绿同一 join 收进矩阵；上报格式 `{完成了什么, 意外点, 尝试过什么, 需要什么}`。
