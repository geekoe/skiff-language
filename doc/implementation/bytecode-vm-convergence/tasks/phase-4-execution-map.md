# MAP4：Phase 4 rolling execution map

> Status: active; revision 1
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

## 3. 验证与纪律

- cargo 租约 `/tmp/skiff-p4-cargo-lease`（mkdir 抢租/rmdir 释放）；focused 每轮跑，三包/全量只在 join 点跑；
- Gate 矩阵首日含 P4 全部 scenario（expected-red）+ fmt/clippy 自检 + Phase 1/2/3 全量回归；
- 每扇门转绿同一 join 收进矩阵；上报格式 `{完成了什么, 意外点, 尝试过什么, 需要什么}`。
