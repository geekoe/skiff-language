# Bytecode VM phase execution and acceptance contract

状态：planned；本目录定义 rollout 阶段、验收证据与阶段间 handoff，不修改
[`doc/architecture/`](../../../architecture/) 或 [`doc/reference/`](../../../reference/) 拥有的语义。

总 implementation scope、included commit/hunk manifest 与完成定义仍以
[`../README.md`](../README.md) 为准。本目录把其中的 recommended sequence 收敛为可执行阶段合同。

## 1. 阶段图

```text
Phase 0  baseline ledger + trustworthy Live harness
   |
Phase 1  artifact schema + structural validator
   |
Phase 2  compiler facts + typed lowering + bytecode emission
   |
Phase 3A exact-build deployment owner cut
   |
Phase 3B relocation linker + monomorphization + semantic verifier
   |
Phase 4  minimal production-shaped VM vertical slice
   |
Phase 5  scheduler + adapters + streams
   |
Phase 6A boundary + DB/unwind/callback + Agine VM cutover
   |
Phase 6B GC + value semantics + ConstantHeap + recoverable parity
   |
Phase 7  Actor + Router exact-build lifecycle
   |
Phase 8  production hard cutover + deletion
   |
Phase 9  release acceptance + performance
```

Phase 3A 必须先于 3B：不能在 provider-dependent synthetic `RuntimeAssembly` 上构造一个仅改名的
`DeploymentExecutionImage`。Phase 6A 必须先于 6B：chat/host 主链先完整进入 VM，随后内存语义在已集成链路上
切换，不能把第一次真实 Agine 集成拖到 GC/COW 之后。

## 2. 阶段状态

每个阶段只能使用以下状态：

- `planned`：尚未形成可验收候选。
- `candidate`：所有阶段任务已合入一个精确候选 commit，但尚未完成隔离验收。
- `candidate-pass`：focused、阶段专属和隔离 Live 均在同一候选上通过，可以合并 main。
- `complete`：候选已合并到各自 main，stable Live 在精确 merge commits 上再次通过。
- `blocked`：存在改变架构语义、需要新授权或无法由当前 scope 唯一决定的问题。

`candidate-pass` 不是阶段完成。Stable 失败会重新打开本阶段；下一阶段不得从失败的 merge checkpoint 开始。

## 3. 每阶段共同验收层

每个阶段都必须完成以下四层证据。阶段页只增加本阶段特有内容，不能删减共同层。

### 3.1 G0：精确候选与 preflight

记录并验证：

- `skiff`、`internals`、`skiff-packages` 的绝对路径、commit、tree hash、branch 和 dirty 状态；阶段最终候选必须
  clean，main checkout 必须仍在 main；
- Rust/Node/npm/pnpm 版本和共享 `CARGO_TARGET_DIR`；Cargo 命令串行执行；
- 实际 compiler、router、runtime 二进制绝对路径与 SHA-256；
- artifact root、profile、动态端口、临时 Mongo 和所有 Live target；
- 本阶段 requirement ledger 的 open/closed/retirement-only 数量。

任何二进制来源不明、compiler SHA 与 manifest 不一致、稳定端口被 worktree 复用或候选在 gate 后发生写入，
都使后续证据无效。

### 3.2 G1：focused 与非 Live 集成 gate

阶段页列出的 selector/测试必须在同一候选上通过。失败只能按以下方式处理：

1. 修复后重新运行受 write set 影响的 focused gate；
2. 若失败可在本阶段基线 commit 重现，记录精确复现命令、日志摘要和 baseline commit；
3. 不得用“与本阶段无关”、skip、零测试、`--no-*` 或旧结果把失败标绿。

阶段最终 acceptance owner 必须至少运行 `git diff --check`，并检查新增 crate 已归入唯一 verify subject。

### 3.3 G2：阶段专属新路径证明

每个阶段必须证明本阶段新增的 production-shaped 路径真的被执行。单元测试、类型存在、未被调用的抽象、
或只验证旧 evaluator 的 Live 结果都不能替代该证据。

从 Phase 2 开始，evidence manifest 必须记录 artifact schema/ISA；从 Phase 4 开始，还必须记录每个被测
deployment 的 execution engine 与 VM counter。VM 验证或执行失败必须直接失败，不得回退 tree evaluator。

### 3.4 G3：每阶段强制隔离 Live

Phase 0 必须实现并注册一个组合 managed selector，canonical 名称为：

```bash
node scripts/verify.mjs --only router-live:agine
```

它在同一个隔离 artifact root、临时 Mongo、真实 compiler、真实 Router/Runtime 和动态 ingress 上依次运行：

1. Agine `chat-smoke`；
2. `host-tools --check`；
3. strict full `host-tools`。

每个阶段都必须运行这个 selector。成功至少满足：

- chat terminal 为 completed，assistant reply 满足 canonical smoke expectation；
- host online/binding成功；full host-tools terminal为completed、assistant非空、至少一个允许的
  `host.file.*` tool call；
- full host-tools 不允许 `host.shell.run`，workspace 收窄到本阶段声明的只读测试根；
- host-tools 使用 harness 显式传入的 runtime PID；profiling 阶段还要求 sample 文件非空；
- host-tools 验证只运行正常流程：同一候选上 chat-smoke、`host-tools --check`、strict full
  `host-tools` 各一次，不多跑角度/注入的 CLI 级对话；注入与多角度验证
  （terminal error、stopped、空答案、零 tool call、错 PID、缺 sample）由 host-tools 单元测试
  （`client/e2e/host-tools-strict.test.mjs`）覆盖同一 strict 断言路径，不是 CLI gate 要求；
- manifest pin 三仓 commits、compiler/router/runtime SHA、全部 deployment/package identities；
- 从 Phase 4 开始，manifest 还必须证明阶段要求的请求进入 VM，不能只看到成功响应。

外部 provider、网络、额度或 secret 不可用可以标记为 infrastructure failure，但阶段不能因此 PASS；修复外部
条件后必须在同一候选或新 evidence epoch 上重跑。

Phase 0 完成前，bootstrap 使用现有 `router-live:chat` 加人工严格 host-tools 断言；这组结果只形成 baseline，
不能替代 Phase 0 最终组合 selector PASS。

## 4. 合并 main 后的 stable closure

各仓库独立提交并按依赖顺序合并 main。重建和发布必须确认 watch 使用本次 compiler；不得调用已经不存在的
独立 activation/fallback 流程。然后在 `internals/agine` main 上运行：

```bash
npm run e2e:chat-smoke
npm run e2e:host-tools
```

Stable host-tools 使用与隔离 gate 相同的严格断言。阶段 result 必须记录：

- 三仓精确 main merge commits；
- release pointer 指向的 Agine/AIHub/Codex Relay buildIds；
- Router health 中的 loaded buildId/image facts；
- chat 与 host-tools 的开始/结束时间、结果和日志摘要；
- 如本阶段修改 Runtime/Router，实际重建和重启的二进制 SHA。

只有 stable closure PASS 后阶段才标为 `complete`。

## 5. Evidence epoch 与结果文档

每个阶段实施时新增 `results/<phase-id>.md`，至少包含：

```text
Status: candidate-pass | complete | blocked
Candidate commits/trees
Requirement IDs closed/deferred
Focused gate table: command, start/end, exit, evidence path/hash
Phase-specific proof
Isolated Live manifest path/hash and assertions
Stable merge commits and Live receipt
Legacy/fallback reverse-search ledger
Performance/layout deltas when applicable
Known residual risks owned by the next phase
Verdict
```

以下事件开始新的 evidence epoch，并使之前的下游结果失效：

- 任一候选仓库 commit/tree变化；
- compiler、artifact schema/ISA、router或runtime二进制变化；
- Live harness、assertion或fixture变化；
- 修复触及已通过 gate 的 production owner；
- artifact root 或 release pointer 被另一轮构建覆盖。

不得把不同 commits、不同 artifact roots 或修复前后的日志拼成一次阶段 PASS。

## 5.5 阶段执行流程要求（Phase 1 复盘沉淀）

以下要求来自 Phase 0/1 实际踩坑，各阶段实施与 Live 验收必须遵守：

1. **Live 前置基线健康检查**：跑隔离 Live 前，先在稳定 dev 上跑一次 `npm run e2e:chat-smoke`（约 1 分钟）确认基线健康。
   基线坏（模型不响应、chat 卡住）时先修环境，不要直接跑隔离 Live——否则 Live 失败难以判别是候选问题还是环境问题。
   Phase 1 的稳定栈故障（dev profiler 开启导致 chat 全断）就是靠这条才快速定位的。
2. **host-tools 失败先重跑判别**：strict full host-tools 依赖真实 LLM，存在模型行为波动（同候选一次 46 次工具调用空回复、
   一次 10 次调用正常）。失败时先在**同一候选**重跑一次判别，不要直接深挖日志/链路。
3. **改 public 结构（字段/序列化）前先全局审计**：先 `rg "TypeName {"` 全仓找构造点，再用
   `cargo check --workspace --all-targets` 一次暴露全部编译错误；批量脚本改代码前先验证匹配规则
   （负向后行断言排除复合类型名与函数签名，`-> TypeName {` 不匹配），并抽查 diff 后再提交。
   Phase 1 的 `PackageArtifact.bytecode` 接线有 36 处构造点，脚本曾两次误伤无关类型。
4. **共享 cargo target 陷阱**：多 worktree 共用 `~/.skiff-cargo-target`，cargo 对 fingerprint 相同的源码复用
   编译产物，`env!("CARGO_MANIFEST_DIR")` 会指向**最近编译该 crate 的 worktree**。任何"写文件"类操作
   （如 `UPDATE_*` 重生成 fixture）执行前确认目标文件在预期 worktree。
5. **工兵任务原子化**：派发的实现任务必须有界、可独立提交（单文件集、自验证）；任务被中止后立即派收尾工兵，
   不要留半成品工作区。
6. **诊断先确认数据源**：telemetry 可能走 4002（存 `skiff_telemetry` 库）、旧 JSONL 落盘或 mongo 加密存储
   （`mongosh` 直读只见 `_id`）；诊断前先确认事件实际写入哪，避免看错日志。
7. **稳定 dev profiler 约定**：稳定 `runtime.yml` 的 `profile.enabled` 必须保持 `false`（见 skiff `AGENTS.md`
   "本地开发"节）；需要 profiling 时用隔离实例，不在稳定 dev 上开启。

## 6. Migration-only 双路径规则

Phase 2–8 可以存在迁移期旧路径，但必须满足：

- 新旧执行格式按整个 deployment/显式测试目标分隔；
- 不存在 opcode、函数、frame 或 verifier-error fallback；
- VM 不调用 tree evaluator，tree evaluator 不进入 VM frame；
- 每个迁移 deployment 的 engine 在 manifest 中显式可见；
- 一旦某 deployment 在阶段内切到 VM，该阶段后续 gate 不得把它切回 legacy 以获得 PASS；
- Phase 8 删除 production tree reader/evaluator和迁移开关；Phase 9 不接受“关闭但仍保留”的旧代码。

## 7. 阶段索引

| 阶段 | 文档 | 阶段专属主证据 |
| --- | --- | --- |
| 0 | [Baseline ledger and Live foundation](phase-0-baseline-live.md) | trustworthy combined gate + frozen baseline |
| 1 | [Artifact schema and structural validator](phase-1-artifact-schema.md) | bounded/malformed/deterministic schema proof |
| 2 | [Compiler facts and emission](phase-2-compiler-emission.md) | real source -> bytecode artifact |
| 3A | [Exact-build deployment owner](phase-3a-deployment-owner.md) | provider-independent per-build image owner |
| 3B | [Linker and semantic verifier](phase-3b-linker-verifier.md) | concrete verified linked image |
| 4 | [Minimal VM vertical slice](phase-4-minimal-vm.md) | real unary request executes only in VM |
| 5 | [Scheduler, adapters and streams](phase-5-scheduler-streams.md) | actual-Pending/flat child/stream proof |
| 6A | [Boundary and Agine VM cutover](phase-6a-agine-cutover.md) | chat + host closure entirely VM |
| 6B | [Heap, values and recoverable](phase-6b-heap-values.md) | GC/COW/const/resource semantic proof |
| 7 | [Actor and Router exact-build](phase-7-actor-router.md) | exact-build Actor lifecycle Live |
| 8 | [Hard cutover and deletion](phase-8-hard-cutover.md) | zero production legacy path |
| 9 | [Release acceptance and performance](phase-9-release.md) | full verify + benchmark + stable rehearsal |
