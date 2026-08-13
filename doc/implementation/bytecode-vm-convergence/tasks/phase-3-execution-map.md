# MAP3：Phase 3 rolling execution map

> Status: active; revision 3; C3 converged union/catch/throw-site, string-literal discriminator slice decided
>
> Phase Contract: [`phase-3-outcome-unwind.md`](../phases/phase-3-outcome-unwind.md)
>
> Phase 2 input: [`phase-2.md`](../results/phase-2.md) accepted on `main`
>
> Baseline commit: `0fee73cdb25dd895ff674646bc25bf61b78d52de`
>
> Integration branch/worktree: `codex/bcvm-p3-integration` / `/Users/geek/workspace/skiff-bcvm-p3-integration`

## 1. Activation receipt

Baseline 是已接受 Phase 2 result 的 `main` tip（`0fee73cd`），merge `41ee6355` 含 accepted candidate
`d0b0b694`/tree `5d5698e9` 与 Phase 2 acceptance receipt。Phase 1（24 命令）与 Phase 2（33 命令）Gate 矩阵全部
成为 Phase 3 的永久回归。main checkout 保持 `main`、clean、与 origin 同步。

## 2. Target and containment ledger

Phase 3 只把同步普通 throw/catch/rethrow（payload 为 Phase 2 面）从 `disabled` 迁到 `accepted`。host effect、
Pending/child/stream、service error 恢复、platform error envelope、async rethrow 保持 `disabled` 且 fail closed。

## 3. Initial ready frontier

三个 leaf worktree 直接建在 `/Users/geek/workspace` 下，从 MAP3 commit 出发；agents 无父对话，自行读契约、
Phase 1/2 receipts 与本 MAP。首日并行，写面不重叠。

| Lane | Role | Worktree | Exact write ownership | First checkpoint / handoff |
| --- | --- | --- | --- | --- |
| K3 | central kernel | `skiff-bcvm-p3-kernel` | `runtime/vm/src/{fiber.rs,lib.rs,error.rs}`（envelope/outcome/unwind）、`runtime/model/src/{service_error.rs,request_heap.rs}`（如 envelope API 需要）、`runtime/scheduler/src/{bytecode.rs,stream_driver.rs}`（ResumeOutcome 投影）、`runtime/request/src/*`（Throw→user error 投影，仅边界相关文件）、`runtime/linker/src/bytecode/link/*`（throw/catch admission）及上述 inline tests | 10m / 40m |
| C3 | compiler lane | `skiff-bcvm-p3-compiler` | `compiler/emission/src/bytecode/{functions.rs,admission.rs,plans.rs}`（throw/catch 发射与 admission）、`compiler/source/src/callable_effects/*`（throw/catch 效果）及 inline admission tests | 8m / 30m |
| P3G | Proof + Gate | `skiff-bcvm-p3-proof-gate` | 新 `runtime/host/src/host/request_entry/phase_3_proof_support*`、`phase_3_vcp_tests.rs`、`scripts/lib/bytecode-vm-phase-3-*.mjs`、`scripts/run-bytecode-vm-phase-3-gate.mjs`、`scripts/tests/bytecode-vm-phase-3-gate-*.test.mjs` 与 selector 注册 | 8m / 30m |

Integrator 只做机械合流、receipt/MAP 更新、Gate/freeze/Acceptance 编排；不补 envelope、默认值、第二 API 或
静态类型回退。K3 是唯一 kernel owner。

## 4. Task contracts

### K3 — exception envelope and outcome kernel（契约 §3.1/3.2）

- `UnwindState` 携带完整 `RequestException`；`execute_throw` 由被抛值运行时 `catch_identity()` 构造
  `RequestException::local`（失败 = VmFailure）；`execute_rethrow` 复用原 envelope；`resume_throw` 消费
  `ResumeOutcome::Throw(RequestException)`；
- 根级未捕获 throw 以 typed outcome 传出，删除 `VmError::UnhandledThrow`；scheduler 不再把普通 throw 压成
  terminal failure；request 驱动把 `Throw` 投影为 canonical 用户 error、terminal 一次；
- `begin_unwind` 每层 frame-exit 走 Phase 2 lifecycle executor；catch 按 actual `CatchIdentity` 匹配；
- linker admission 放行同步 throw/catch/rethrow，host/Pending throw fail closed；
- 不改变 Phase 1 观察/budget/terminal/cleanup 与 Phase 2 lifecycle/COW 语义。

### C3 — throw/catch emission and admission（契约 §3.3）

- emission 不写死 `payload_type`；union/nominal 异常保留 actual identity 运行时捕获所需的 facts；admission
  放行同步 throw/catch/rethrow（payload 为 Phase 2 面），其余 fail closed；缺失 fact 稳定拒绝、不发布 artifact；
- 不扩大 host/Pending 支持面。

### P3G — expected-red VCP-3 + negative + Gate（首日含全部 scenario）

- VCP-3：union A/B 叶 catch 匹配、rethrow identity、cleanup owner drop（heap spy）、terminal 恰一次；受控
  resume harness 证明 identity；首日 expected-red；
- negative：A 叶不匹配 `catch<B>`、未捕获 throw 投影 user error、host/Pending throw 拒绝；
- Phase 3 Gate 含 Phase 3 场景 + Phase 1/2 全量回归（复用 phase-1/2 workload specs，不重发明）；Node 自测用
  合成 receipt，拒绝 dirty/stale/missing/zero/skip/tamper；producer 转绿同一 join 收进矩阵。

## 5. Integration and validation order

1. C3 先合（不依赖 K3），K3/C3 producer-consumer join 一起跑 VCP；
2. cargo 共享 target `/Users/geek/workspace/.skiff-cargo-target`；跨 worker 用 `/tmp/skiff-p3-cargo-lease` 目录
   租约串行（mkdir 抢租/rmdir 释放/sleep 5 轮询），每条 cargo 必须持租；
3. >30s 命令重定向 `/tmp` 轮询，不复跑找回输出；
4. 每个 join 后跑受影响的最小 preflight；P3G Node 自测可随时跑；
5. merged preflight 全绿后 freeze、detached Acceptance worktree、全新 Acceptance Agent 跑完整 Phase 3 Gate。

## 6. Watchdog and takeover

checkpoint = 可见 code/test/decision 输出 + 当前 blocker。首 checkpoint 按 §3 表；超 15 分钟无 diff 要求部分提交
或停止；30 分钟无可信 handoff 打断并由新 owner 从最后可信 commit 接管。K3 永不拆 owner；read-only 诊断可并行。

## 7. Candidate and evidence epochs

integration line 不是 acceptance 候选。全绿后 integrator 跑 merged preflight、freeze exact commit/tree、创建
detached Acceptance worktree；此后任何 production/test/fixture/Gate/schema 变化开新 epoch。只有全新 Acceptance
Agent 可出最终 verdict。

## 8. Revision 2

- C3 `1a2e1882`/`5d94d1cd`：Throw operand 去静态 payload_type（收敛为 value-derived transfer fact）、同步
  throw/catch/rethrow admission 放宽（union 叶、异常 region 精确 fact、host/Pending throw 拒绝），6 个新测试。
- K3 `e7354a3a..ef680916`（共 5 commit）：`UnwindState` 携带 `Arc<RequestException>`+cursor+phase；throw 从运行时
  叶 tag 派生 `CatchIdentity`；rethrow 复用同一 Arc；resume_throw 两阶段 unwind；`VmError::UnhandledThrow` 删除、
  `VmError::Thrown` typed outcome；scheduler/request 投影（Throw→user error、envelope VmFailure→sanitized
  InternalError、PlatformTerminal 不变）；linker 放行 Throw/Rethrow/异常 region/匿名 Union。三包 301 全绿。
  `runtime/vm/src/control.rs`（35 行，ResumeOutcome::Throw 类型与 VmRootSource）按上报阈值记入 K3 写集。
- P3G `6d2e84c3`/`071bc917`：VCP-3 + 4 negative expected-red、受控 resume substrate、Phase 3 Gate（12 probes +
  34 workloads：13 P3 场景 + 12 P1 回归 + 9 P2 回归），Node 113/113 绿。
- 三 lane 已滚入 integration（`fb13148b`/`11a712b6`/`4980d316`）。P3G 在合并树上重跑找真实剩余红；join 契约
  filter：`k3-*`/`c3-*` 由 P3G 的 contract.mjs 指定。

## 9. Revision 3

- P3G 跟进 `7871f97c`/`cfb2c030`（fixture 收敛、K3 投影断言、受控 resume 对齐）；C3 跟进 `dd319a5e`：union
  分支构造改写为外层匿名 union（runtime identity 仍叶）、rethrow expression-key 漂移修复、`CatchResult<never,E>`
  槽类型、Throw 指令 source/synthetic site。uncaught/host/Pending negative 已绿。
- 剩余红：mismatch fixture 的 `attempt.tag == "ok"` 因 Phase 2 无 string 面被 admission 拒绝。契约 §4a
  Amendment 1 定案：放行编译期 string literal 仅作 discriminator 常量（tag 相等与 string-literal-union 匹配），
  通用 string 值仍 fail closed。授权 C3 收 admission/emission，K3 收 linker/VM 常量比较（如需）。

## 10. Revision 4

独立 reviewer REV3 判 FAIL，三项 routing 后补记（修订非重做）：

1. **写集扩展补记（F1）**：此前经 integrator follow-up 授权、但未写入 MAP3 的文件组，现正式记录为各 lane 写界：
   K3 = `runtime/linker/src/bytecode/stack_map/*`、`runtime/bytecode-verifier/{src/control_flow/**,src/concrete_values/*}`、
   `runtime/request/src/bytecode_ingress.rs`（投影）、`runtime/vm/src/{fiber.rs,error.rs,control.rs}`；
   C3 = `compiler/source/src/{expression_type_model*,assignability*,value_transfer*}`、
   `compiler/lowering/src/*`、`compiler/emission/src/bytecode/{functions,admission}.rs`。逐 commit 均可追溯。
2. **受控 resume 闭环（F2）**：K3 补一个 live VM 测试，把 `ResumeOutcome::Throw` 真实送入 `resume_throw`
   （经 `set_error_correlation`），断言 envelope identity 跨 resume 不变。
3. **名义叶-only throw 面（F4）**：契约 §4b Amendment 2——emission admission 稳定拒绝 structural/scalar/literal
   叶 throw（运行时恒 VmFailure 的面移回 fail closed）；literal-branch identity 记后续义务。
4. F3（linker/verifier union-branch 接受面不对称，verifier 更严即 fail-closed）为非阻塞 advisory，保留记录。

Gate preflight 已 PASS（46/46、267/267）；上三项闭合后重跑 preflight → freeze → 全新 Acceptance。
