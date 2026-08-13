PASS

# Phase 3（outcome & unwind）第三轮独立 Acceptance receipt

> Acceptance Agent：第三轮全新独立，未参与本 Phase 任何 production/test/Gate 写入。全程只读：候选
> worktree 未创建/修改任何文件、未 commit；唯一写入是 Gate 证据树与本 receipt（均在
> `/Users/geek/workspace/skiff-bcvm-p3-acceptance-evidence-r3/`，候选之外）。工具链与 R0 基线一致：
> rustfmt `1.8.0-stable (6b00bc3880 2025-06-23)`、cargo/rustc `1.88.0`、node `v22.17.0`。

## 0. Verdict

**PASS**。canonical Gate 全绿、契约 §7 可验收项 1–10 全部有据支撑、前两轮的唯一 blocker（Phase 3
写入行引入 rustfmt 新红）已完全闭合：`runtime/request/src/bytecode_ingress.rs` 的 R2 残留（hunk 155）
在冻结候选 `411da455` 中消失，且 Phase 3 区间（`0fee73cd..411da455`）改动的全部 33 个 `.rs` 文件
rustfmt 零新红残留（27 个零 diff；6 个残留 hunk 逐行 blame 全部落在 baseline 之前，旧漂移）。
本 receipt 不修复、不修改候选、不 commit，原样报告。

## 1. Candidate 与 evidence identity

- 候选 commit：`411da4559ec2e61df525e5dd8baf68fac5175e45`；tree：`a8c225df65d471f50cf13eb18bd01551168b216b`
  （detached worktree `/Users/geek/workspace/skiff-bcvm-p3-acceptance`，`git rev-parse HEAD/HEAD^{tree}` 复核
  一致；`git status --porcelain=v1 --untracked-files=all` 在 Gate 前、Gate 后、本审查后均为空）。
- 候选修复 commit（`git show --stat` 确认仅 2 文件，54+/55-）：`fix(runtime): rustfmt the remaining Phase 3
  bytecode_ingress fmt residual`——`runtime/request/src/bytecode_ingress.rs`（纯格式化）+ MAP3 状态头
  更新为 `revision 6`。
- Gate 证据根：`/Users/geek/workspace/skiff-bcvm-p3-acceptance-evidence-r3/gate`
  - `manifest.json` SHA-256 `2c8d1ff67e765a114424edafa34bb98e5e242426487ed098f364853814d9559e`
  - `phase-3-directory-identities.json` SHA-256
    `75b01a52a1988ae55776b551fff6e1ef0a36e3fd3668a4632f8a31899eee748f`
- Gate 运行日志：`/tmp/skiff-p3-acceptance-r3-gate.log` SHA-256
  `b4360508bbe2588aa7e6a3d300049b27f9f0191c1e054e72770b6252f5dd370a`
  （2026-08-13T14:16:10.505Z → 14:19:05.468Z，约 2.9 分钟，exit 0）。
- fmt 检查日志：`/tmp/skiff-p3-fmt-r3.log` SHA-256
  `4bdef7168a93e97354d08eb1f7b9b2198641f275cb84727251ed012f68ceb4d0`
- observation schema：`skiff-bytecode-vm-phase-1-observation-v1`，SHA-256
  `88e261ee444e9742683194a2f5592841f070aed6204b04f197eddef3630a4d0e`——与 Phase 1 accepted epoch
  字面量一致；`scripts/lib/bytecode-vm-phase-1-observation-schema.mjs` 在 Phase 3 区间零 diff。

## 2. Canonical Gate 执行

命令（cwd=detached 候选 worktree；`env | grep '^GIT_'` 仅 `GIT_PAGER=cat`，属 allowlist，无 `env -u`
需要；受管会话 + 轮询，单次 bash 约 2.9 分钟 ≤ 5 分钟；启动前确认无 cargo/rustc 进程、无 cargo 租约，
共享 target `/Users/geek/workspace/.skiff-cargo-target`，无并发、无 `cargo clean`）：

```bash
node scripts/run-bytecode-vm-phase-3-gate.mjs \
  --output-dir /Users/geek/workspace/skiff-bcvm-p3-acceptance-evidence-r3/gate \
  --candidate 411da4559ec2e61df525e5dd8baf68fac5175e45 \
  --tree a8c225df65d471f50cf13eb18bd01551168b216b
```

结果：

- `verdict === "PASS"`；`counts.commands = {total: 46, passed: 46, failed: 0}`；
- `counts.tests = {declared: 272, passed: 272, failed/skipped/todo/cancelled/ignored 全 0}`，
  `declared === passed`（与 R1/R2 相同的 272，非漂移）；
- `candidate.exact = true`、`candidate.clean = true`；preflight/postflight/closure/fresh 四快照
  `{commit, tree}` 全部等于候选、`status: ""`；
- `failures: []`；runner 自身输出 `checkerError: null`；
- lane 覆盖全绿：12 快照 + phase-3-gate-self-tests + 5 P3 场景（VCP、3 negative、controlled-resume-harness）+
  k3×5 + c3×2 + phase-1-regression×12 + phase-2-regression×9 = 46 命令全部 PASS。

## 3. 证据闭包独立重算

独立 Node 一遍（不调用 Gate 自带 checker）对 `manifest.evidenceFiles` 逐文件重算 bytes + SHA-256：
**139 文件，0 偏差，0 缺失**；磁盘上除 `manifest.json` 自身（不能自含哈希）外无未登记文件。
`phase-3-directory-identities.json` 与 `manifest.request.directoryIdentities` 解析后深相等（
`JSON.stringify` 逐字节相等；磁盘文件为 pretty-print、manifest 内为紧凑序列化，故原始字节不同但语义
全等）。抽查 7 份 workload receipt（`phase-3-vcp-production-composition`、
`phase-3-negative-catch-mismatch`、`phase-3-negative-uncaught-throw`、
`phase-3-negative-host-pending-throw`、`phase-3-controlled-resume-harness`、`k3-vm-throw-unwind`、
`c3-emission-throw-admission`）：均 `outcome = {code: 0, signal: null, error: null, status: "PASS"}`，
`testSummary` 0 failed/ignored 且 `valid: true`。全 46 命令 `testSummary` 汇总 passed=272、total=272、
failed/cancelled/skipped/todo/ignored 全 0，与 manifest counts 一致。

## 4. fmt 闭合检查（本轮重点，独立执行）

`cargo fmt --all -- --check`（exit 1，符合预期）：**550** 个 `Diff in` hunk / **173** 个文件 ≤ R0 旧红
652。Phase 3 区间 `0fee73cd..411da455`（46 commits）改动的 `.rs` 共 **33** 个，其中：

- **27 个文件零 fmt diff**，含前两轮列过的全部关键文件：`compiler/emission/src/bytecode/{admission,
  functions}.rs`、`runtime/vm/src/{fiber.rs,error.rs,control.rs,fiber/tests.rs}`、
  `runtime/model/src/{service_error.rs,request_heap.rs}`、`runtime/linker/src/bytecode/link/capability.rs`、
  `runtime/linker/src/bytecode/stack_map/{mod,transfer,merge,values}.rs`、
  `runtime/host/.../phase_3_{vcp_tests,proof_support,proof_support/fixture,proof_support/request_composition}.rs`、
  `runtime/request/src/bytecode_ingress.rs`（R2 阻断的 hunk 155 消失，零 `Diff in` 条目）、
  `compiler/source/src/expression_type_model/assignability.rs`、`compiler/lowering/src/{mir/tests,
  type_inference}.rs`、`compiler/source/src/value_transfer/{native,tests/plans}.rs`、
  `runtime/bytecode-verifier/src/concrete_values/mod.rs`、`.../instruction/slots.rs`、
  `runtime/scheduler/src/bytecode.rs` 等。
- **6 个文件仍含 fmt hunk**（与 R2「旧漂移 6 文件」清单完全一致），对每个文件做
  `rustfmt --edition 2021 --emit stdout` 规范输出与当前内容逐行 diff（difflib, n=0），再对每条被删行
  `git blame -L n,n 411da455`：**Phase 3 区间命中数全为 0**，全部 blame 到 13 个 baseline
  （`0fee73cd`）之前的旧 commit（逐一 `git merge-base --is-ancestor` 验证为真）：
  - `compiler/lowering/src/function_lowering.rs`：10 行（c63b26fa/30a4f2af/c3280f91/d12aa90e/
    9a7ecc8f/7b56c2ad/5c3b9e38）→ 旧漂移；
  - `compiler/source/src/expression_type_model.rs`：8 行（c3280f91×6/077578d4/46bcddf9）→ 旧漂移；
  - `compiler/source/src/expression_type_model/tests.rs`：1 行（c3280f91）→ 旧漂移；
  - `runtime/bytecode-verifier/src/control_flow/transfer/instruction.rs`：15 行（b7c620e1）→ 旧漂移；
  - `runtime/bytecode-verifier/.../instruction/values.rs`：2 行（662ca38a）→ 旧漂移；
  - `runtime/scheduler/src/stream_driver.rs`：6 行（296462db×5/0ac97bfe）→ 旧漂移。
  - 无纯插入型 hunk（stdin 模式消除 rustfmt 路径头伪 hunk 后为 0），无漏检面。

**Phase 3 写入文件零残留**：33/33 个 Phase 3 `.rs` 文件中不存在任何由 Phase 3 commit 写入且仍带
rustfmt 告警的行。R2→R3 delta 的 `bytecode_ingress.rs` 逐字节等于 `rustfmt --edition 2021` 对 R2 内容
的输出（零手改、纯格式化），R3 版单独 `rustfmt --check` exit 0。

## 5. 反假绿快速复核（沿用前两轮已验结论，本轮只读复核）

- `UnhandledThrow`：`runtime/`、`compiler/`、`test-runner/` 生产代码零匹配（全仓库仅 docs 提及）。
- §4b 名义叶-only throw：`admit_throw_payload_type`（`compiler/emission/src/bytecode/admission.rs:1117`）
  只放行 local/publication nominal record 叶与它们的匿名 union；其余叶 `ValueShape` 稳定拒绝、无运行时
  恒 VmFailure 面。
- §4a 判别符切片：`referenced_constant_indices`（capability.rs:386）、`is_discriminator_string_constant`
  （:599）、`admit_structural_leaf`（:841）门禁仍在；通用 string 值 fail closed。
- 窄 union 分支可赋值：`union_branch_assignable`（concrete_values/mod.rs:132）仅 slot 写
  （slots.rs:168/181）与 call 参数（instruction.rs:401 `require_assignable`）；REV3 F3 advisory 维持。
- live `resume_throw` 测试确在 Gate 矩阵 `k3-vm-throw-unwind` lane 内（stdout 含
  `controlled_resume_throw_preserves_the_exact_envelope_into_the_catch_handler`，该 lane 4/4 全绿；
  REV3 F2 闭合属实）。
- Phase 1/2 不变量：`runtime/vm/src/lifecycle.rs`、`runtime/model/src/bytecode_execution_observation.rs`、
  Phase 1 observation schema 文件在 `0fee73cd..411da455` 区间零 diff；observation schema identity 未变；
  12+9 回归 lane 全绿。

## 6. 契约 §7 checklist

1. [x] envelope 单一权威；throw 用 actual runtime identity；rethrow/resume_throw 保持 identity。
   （`service_error.rs` `VmLocal`；`fiber.rs` runtime leaf identity/`execute_rethrow`/`resume_throw`；
   live `Arc::ptr_eq` 测试在矩阵内）
2. [x] root outcome 四分类；`VmError::UnhandledThrow` 已删（生产代码零匹配）；terminal 恰一次。
3. [x] catch 按 actual `CatchIdentity` 匹配（runtime 叶 TypeIndex）；union 叶 A/B 行为正确
   （VCP + mismatch negative → canonical user error）。
4. [x] unwind 每层 frame-exit 走 Phase 2 `LifecycleExecutor`（`unwind_loop`→`release_frame_exit`）；
   cleanup owner 释放由 spy heap 序列证明。
5. [x] 请求边界三类投影：uncaught → canonical user error；envelope VmFailure → sanitized
   `InternalError`；`PlatformTerminal`/`InternalTerminal` 路径不变（Phase 1 语义）。
6. [x] admission 只放行同步 throw/catch/rethrow（§4b nominal-leaf-only）；host/Pending throw fail closed
   （effects + linker `admit_effect_summary` + negative fixture 稳定 typed 拒绝、无 artifact 发布）。
7. [x] VCP-3 与受控 resume harness 全绿（5 场景 + live resume_throw 测试在矩阵内）。
8. [x] Phase 1（11 事件/budget/terminal/cleanup）与 Phase 2（lifecycle/COW）回归全绿，schema identity 未变。
9. [x] canonical Gate 聚合全部 required evidence class，并拒绝 dirty/stale/missing/zero/skip/tampered
   （Node 自测 + live checker `checkerError: null` + 本 receipt 独立重算 139 文件 0 偏差）。
10. [x] frozen candidate 由本第三轮全新 Acceptance Agent 给出 PASS（本节 verdict=PASS）。
11. [ ] 下游时序（非验收时点判据，沿 Phase 2 receipt 先例）：Phase 3 result 需先合入 `main` 再标
    `accepted`、Phase 4 才解禁。当前 Phase 3 仍 `active`、无 `results/phase-3.md`、无 Phase 4 契约，
    未提前解禁。

Tally：10×[x]，1×[ ]（item 11 为下游义务）。

## 7. Waivers 与 findings

- Waiver 唯一合法项：R0 R-FMT **旧红**（652 条目历史漂移，非本链）。当前 550 hunk 全部为旧漂移，
  Phase 3 写入行零残留（§4），不触发「R0 之外新红」阈值。
- Finding 1（非阻断，记录）：MAP3 状态头已标 `revision 6`（候选修复 commit 更新），但正文最后一节仍为
  `## 11. Revision 5`，无 Revision 6 正文小节；与 R1 Finding 1 同类（文档滞后，不豁免验收项，建议
  integrator 合入时补记）。
- Finding 2（非阻断）：R2 的阻断 finding（`bytecode_ingress.rs:155` Phase 3 新红）已闭合——该文件现在
  `cargo fmt --check` 零 `Diff in`，且 R2→R3 delta 逐字节等于 rustfmt 输出（纯格式化、零手改）。
- Finding 3（非阻断）：REV3 residual notes 属实——根级未捕获 envelope payload 靠 request-heap teardown
  释放（Phase 4 义务）；`vm_error_to_request_error` 的 `Thrown` 防御臂仅无 heap 物化的 lane 可达。
- Finding 4（非阻断）：REV3 F3 advisory（linker/verifier union-branch 接受面不对称，verifier 更严即
  fail-closed）保留记录。

## Verdict

PASS。canonical Gate 46/46、272/272、exact+clean、`failures=[]`、`checkerError=null`；§7 项 1–10 全
支撑、项 11 为下游时序义务；前两轮唯一 blocker（Phase 3 写入行的 rustfmt 新红）在冻结候选上独立复核
完全闭合。waiver 仅限 R0 R-FMT 旧红。原样报告、未修文件、未 commit。
