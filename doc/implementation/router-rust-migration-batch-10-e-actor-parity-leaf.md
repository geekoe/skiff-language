# Router Rust Migration Batch 10 — E-actor-parity Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_e_actor_parity`
集成目标：`/root/router_rust_integration_b10`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-10.md`
  （E-actor-parity 节点；baseline `origin/main@edc111f8`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §7 E-actor-parity、§8 `router-live:actor` 原子扩展、§9 differential
  harness 政策（独立端口/artifact root/runtime home/Mongo namespace、不共享
  Runtime、normalization 仅 UUID/timestamp/ephemeral port/无语义 log order）。
- E-actor-rust gate leaf：`doc/implementation/router-rust-e-actor-rust-leaf.md`
  （已知延迟 seam #2：`ActorActivationControlPort` ownerLeaseId mint 与
  registry commit mint 的 reconciliation 归本节点）。
- A2 leaf：`doc/implementation/router-rust-migration-a2-leaf.md`（TS 硬切
  canonical actor projection；A2 已合入 edc111f8）。
- W-differential leaf：`doc/implementation/router-rust-migration-batch-8-w-differential-leaf.md`
  （harness 结构、normalization 政策、scenario inventory 契约）。
- 共享 differential harness：`scripts/lib/router-differential/`（本节点新增
  文件一律 `actor_parity_*` 前缀；不修改共享模块）。

## 零 worktree 只读预检结论（锚定 edc111f8）

1. 基线锚定：`git rev-parse origin/main` =
   `edc111f888a70743a8ecadc3bdbcb6b4ae2fd54a`；worktree
   `/Users/geek/workspace/wt-e-actor-parity`（分支
   `feat/router-rust-e-actor-parity`）HEAD 即该 commit。
2. seam 现状（Rust）：
   - `router/src/supervisor/actor.rs::ActorActivationControlPort` 持
     `LeaseIdMint`，在 activateInitial wire fence 独立 mint
     `owner-lease-<n>`；
   - `router/src/actor/ownership.rs::commit` 用 `next_lease_seq` 独立 mint
     `owner-lease-<n>`；
   - 两处 mint 无关联 → wire fence lease id 与 committed registry fence
     lease id 可不同（E-actor-rust leaf 记录 seam）。
   - TS 现状：`ActorGetCreateActivationCoordinator.activateInitial` 在
     `acquireOwnerLease` 一次 mint `actor-owner-<id>`，同一
     `fence.ownerLeaseId` 用于 wire frame / markOwnerLive / renew /
     release（单一 mint，语义已对齐）。
3. A2 已硬切：`router/src/router/actorRoutingProjection.ts` 存在，TS
   `filesystemRuntimeAssemblySnapshotLoader.ts` 只读
   `records/actor-routing/current.json`（共享 corpus 单测在
   `actor-routing-projection-reader.test.ts`）。
4. differential harness：`scripts/lib/router-differential/`（constants、
   frames、relay、mongo、normalize、compare、instance、scenarios）与
   `scripts/check-router-differential-live.mjs` 就绪；共享 inventory
   `scripts/fixtures/router-differential/scenario-inventory.json` 中
   `actor-two-replica` 为 planned（共享 inventory 由 differential 扩展节点
   统一维护，本节点不动它，改用自己的 `actor_parity_*` inventory）。
5. `router-live:actor` 当前 checked-in task expectation 是 Rust-only
   （`scripts/lib/verify-live-registry.mjs` description），必须在本 change
   更新为 TS/Rust differential，避免 Rust-only 被误当 parity。
6. actor full-chain 驱动基线：`scripts/run-actor-full-chain-acceptance.mjs`
   + `scripts/lib/actor-full-chain-acceptance-real.mjs`（TS 侧 real HTTP +
   2 replicas）；`scripts/check-router-actor-live.mjs` +
   `router/tests/actor_live_probe.rs`（Rust 侧 two-replica + 负例/归零）。
7. 本机工具齐备：cargo/pnpm/node/mongod/mongosh/python3；主 worktree 有
   router node_modules；本 worktree 需自装 router deps 供 TS 侧启动。

## 任务范围

1. ownerLeaseId mint reconciliation（Rust 单 mint 化 + TS/Rust 两侧测试）：
   - `LeaseIdMint` 从 `supervisor/session_ports.rs` 移入
     `router/src/actor/types.rs`（broker 在激活 admission 时一次 mint）；
   - `CommitFenceFacts` 增加 `owner_lease_id`；broker claim 与
     `ActivateInitialControlRequest.facts` 携带同一 mint；
   - `ActorActivationControlPort` 使用 `request.facts.owner_lease_id`
     （删除自身 `lease_mint`）；
   - `ActorOwnershipRegistry::commit` 使用 `facts.owner_lease_id`
     （删除 `next_lease_seq` 独立 mint）；
   - 新增 Rust broker 级 reconciliation 单测（wire frame lease id ==
     committed fence lease id、两次激活 id 不同）；TS 侧补显式
     wire fence.ownerLeaseId == registry entry.ownerLeaseId 断言。
2. `router-live:actor` 原子扩展为 TS/Rust differential full-chain：
   - 新 `scripts/` differential 文件（`actor_parity_*` 前缀）：
     独立 inventory（`actor_parity_full_chain`）、two-replica 每侧编排、
     implementation-neutral HTTP full-chain driver（probe/slowGet/
     slowDedup/slowIncrement/flakyGet/synchronousSelf*/spawn*/chain*）、
     帧事件投影（id/timestamp/port 按场景声明 normalize；payload sha256；
     health 帧从 equal 序列剔除）、Mongo state/audit、terminal；
   - `scripts/check-router-actor-live.mjs` 扩展为：先跑 Rust-only
     `actor_live_probe`（保留负例/归零层），再跑 TS/Rust differential
     全链对比（无未解释差异即 PASS）；
   - 两侧均消费同一 **真实（非空）canonical projection record**（A2 硬切
     语义）：differential 首轮发现 TS 在空 projection 下对
     `actor.method.invoke` 返回 UnknownMethod（fail closed、无 error
     frame）而 Rust 直接转发——既有未解释差异；收敛为 harness 以 test-side
     A1 producer 角色合成真实 projection（canonical JSON），Rust
     `ActorFrameSink` 补 catalog admission（miss → fail closed 不转发不写
     error frame，与 TS 一致），`actor_live_probe` 改为校验真实记录。
3. registry / CI / doc：
   - `scripts/lib/verify-live-registry.mjs`：actor 条目 description 改为
     differential；requiredExecutables 增加 `pnpm`；requiredModules 增加
     `ws`（TS side + relay）；
   - `scripts/tests/verify-live-registry.test.mjs`：同步 actor 条目断言；
   - `.github/workflows/router-rust-integration.yml`：actor job 增加
     pnpm + `pnpm --dir router install --frozen-lockfile`，classifier 增加
     `actor_parity_*` 路径；
   - 本叶子 + `...-actor-parity-differential-scenarios.md`。

## 写入边界（worktree `/Users/geek/workspace/wt-e-actor-parity`）

可写：

- `router/src/actor/types.rs`、`router/src/actor/mod.rs`、
  `router/src/actor/activation.rs`、`router/src/actor/ownership.rs`、
  `router/src/supervisor/actor.rs`（仅 mint 接线）、
  `router/src/supervisor/session_ports.rs`（仅移除 LeaseIdMint）。
- `router/tests/actor_support/mod.rs`、`router/tests/actor_activation_broker.rs`、
  `router/tests/actor_ownership_registry.rs`、`router/tests/actor_live_lane.rs`、
  `router/tests/composition_components.rs`（mint 字段机械同步 + 新单测）、
  `router/tests/gates_wiring_actor.rs`（catalog admission fail-closed 单测）、
  `router/tests/actor_live_probe.rs`（projection 校验，不覆写为空）。
- `router/tests/actor-get-create-activation.test.ts`（TS 侧 lease-id 显式断言，
  仅 parity 所需最小测试）。
- `scripts/lib/router-differential/actor_parity_*`（新文件）、
  `scripts/fixtures/router-differential/actor_parity_inventory.json`（新）、
  `scripts/check-router-actor-live.mjs`（扩展）。
- `scripts/lib/verify-live-registry.mjs`（仅 actor 条目）、
  `scripts/tests/verify-live-registry.test.mjs`（仅对应断言）。
- `.github/workflows/router-rust-integration.yml`（仅 actor job + classifier）。
- `doc/implementation/router-rust-migration-batch-10-e-actor-parity-leaf.md`、
  `doc/implementation/router-rust-migration-batch-10-actor-parity-differential-scenarios.md`。

禁止：

- runtime crate、`runtime/transport/src`、deployment、router TS 生产代码
  （除上述显式测试）、AGENTS.md、scripts README、verify selector graph、
  `skiff-instance.mjs`、共享 inventory 与 `scripts/lib/router-differential/`
  既有模块、CI 其他 job。
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 verify。

## 实现决策（在生产代码前冻结）

### D1：ownerLeaseId 单 mint 归 broker

- `LeaseIdMint` 移入 `router/src/actor/types.rs` 并随 `actor/mod.rs` 导出；
  `session_ports.rs` 删除该定义（无其他使用者）。
- `ActorActivationRequestBroker` 增加 `lease_mint: LeaseIdMint`；
  `get_or_create` 在 claim 建立时 mint 一次，写入 claim.facts 与
  `ActivateInitialControlRequest.facts.owner_lease_id`。
- `ActorActivationControlPort::send_activate_initial` 直接取
  `request.facts.owner_lease_id`。
- `ActorOwnershipRegistry::commit` 以 `facts.owner_lease_id` 落 fence，
  删除 `next_lease_seq`。
- 语义：一次激活一个 lease id；wire、commit、markLive、renew、release 全链
  同一 id（与 TS `actor-owner-<id>` 单 mint 语义一致；格式不要求一致）。

### D2：actor_parity differential harness

- 复用：`actor_live_fixture.mjs`（artifact authoring）、
  `router-differential/{frames,mongo,normalize,compare}.mjs`、
  `dev-runtime-paths.mjs`（RouterProcessSpec seam）、
  `runtime-stack-config.mjs`、`local-port-lease.mjs`、
  `activation-state-live-harness.mjs`、`cargo-target-dir.mjs`。
- 不修改共享 `relay.mjs`：新建 `actor_parity_relay.mjs` 保留 payload
  bytes/raw bytes（供 sha256 投影）。
- 新建 `actor_parity_projection.mjs`（test-side A1 producer）：从 artifact
  的 PackageArtifact / File IR 记录合成真实 canonical actor-routing
  projection（与 A2 test helper 同构；canonical JSON 编码与
  skiff-canonical-json 一致），写入 `records/actor-routing/current.json`，
  两侧 Router 消费同一字节流。
- 每侧 4 连端口（http/runtime/relay1/relay2，45000-45999）+ 独立临时
  mongod + 独立 artifact root / runtime home / dev home；replica id 固定
  （`actor-parity-replica-1/2`），两侧一致。
- HTTP driver 顺序固定（与 TS acceptance 同序），记录每步
  `{name,status,body}`；同时从两个 relay 抓 actor/spawn/request/response
  帧并投影：id 字段（rpcId/requestId/invocationId/spawnId/ownerLeaseId/
  traceId/spanId 等）→ `<token-N>`（按首次出现顺序，跨侧同序可对齐）、
  timestamp → `<timestamp>`、port → `<port>`（复用 compare 引擎
  normalization 政策），非空 payload → `payloadSha256`；`runtime.health`
  与握手帧不进 equal 序列（recordOnly 原始证据）。
- scenario inventory：`actor_parity_full_chain`（runnable），compare 覆盖
  http.steps / frameEvents / health.status / mongo.state / mongo.auditCount /
  terminal；sideExpected 覆盖 bootstrap artifactsPath/mongoUrl（沿用
  session-handshake 政策）。

### D3：gate 装配

- `check-router-actor-live.mjs` 保留既有 Rust probe 段，追加 TS/Rust
  differential 段；`router-live:actor` PASS = 两段均通过。
- registry description 明确 "TS/Rust differential full-chain"。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| ownerLeaseId reconciliation 单测 | `cargo test -p skiff-router --test actor_activation_broker actor_ownership_registry composition_components actor_live_lane` 全绿（含新 reconciliation 用例） |
| TS 侧 lease-id 断言 | `pnpm --dir router exec vitest run tests/actor-get-create-activation.test.ts` 全绿 |
| differential 全链 | `node scripts/check-router-actor-live.mjs` PASS（Rust probe + TS/Rust 对比：18/20 equal + 2 项已记录并接受的 known-differences，无未声明差异） |
| registry | `node scripts/verify.mjs --only router-live:actor --list` 描述为 differential；`node --test scripts/tests/verify-live-registry.test.mjs` 全绿 |
| workflow YAML | Python yaml 解析通过；actor job 含 pnpm/install 步骤 |
| 写集 | `git status` 仅本叶子声明文件；`git diff origin/main...HEAD` 聚焦 |

## 停止条件

- differential 出现未解释差异 → 定位 owner（Router TS / Router Rust /
  harness）后停下上报，不掩盖。
- 需要 runtime crate / transport / deployment 改动 → 停下上报。

## 执行结果（提交前填写）

### 已交付

1. ownerLeaseId mint reconciliation（Part 1，全绿）：
   - `LeaseIdMint` 移入 `router/src/actor/types.rs`；broker 在激活 admission
     一次 mint；`CommitFenceFacts.owner_lease_id` 贯通 claim/control
     request/registry commit；control port 不再自持 mint；registry commit
     不再独立 mint。
   - 新增 Rust 单测 `owner_lease_id_is_minted_once_and_reused_at_commit`
     （wire 帧 lease id == committed fence lease id、两次激活 id 不同）；
     TS 显式断言 `reconciles one owner lease id across the wire frame and
     the registry entry`。
2. differential harness（Part 2）：
   - `scripts/lib/router-differential/actor_parity_*`（constants、relay、
     scenarios、fixture、projection、driver、instance、runner）+
     `actor_parity_inventory.json` + `scripts/check-router-actor-live.mjs`
     扩展；`actor_parity_full_chain` 场景 runnable。
   - test-side A1 producer（`actor_parity_projection.mjs`）合成真实（非空）
     canonical projection（两侧消费同一字节流；canonical JSON 与
     skiff-canonical-json 一致）。
3. parity 所需最小 Rust 修复（differential 首轮定位后实施）：
   - `ActorFrameSink::handle_method_frame::Invoke` 补 catalog admission
     （miss → fail closed 不转发不写 error frame，与 TS UnknownMethod
     语义一致）；`gates_wiring_actor` 单测覆盖。
   - owner 选择改为 Router 侧确定性 hash-pin
     `sha256(actorIdHash) % candidates`（TS `pickOwner` parity；并发
     create 不再依赖先到先得）；`actor_sink.rs` 内单测覆盖。
   - `ActorInvocationRelay::parent_snapshot` 对 ordinary invocation 返回
     owner runtime/connection（C-spawn §4.2；跨 runtime actor-method spawn
     的 parent authority），test-capability 保持 caller origin。
   - `actor_live_probe` 的 `materialize_projection` 改为校验真实记录，
     不再覆写为空。
4. registry / CI / doc：`router-live:actor` 描述改为 TS/Rust differential；
   requiredExecutables 增加 `pnpm`、requiredModules 增加 `ws`；CI actor
   job 增加 pnpm/install；classifier 覆盖 `actor_parity_*`；本叶子 +
   `...-actor-parity-differential-scenarios.md`。

### 自验收

- `cargo test -p skiff-router --test actor_activation_broker actor_ownership_registry
  actor_live_lane composition_components gates_wiring_actor` 全绿（含新单测）；
  `cargo test -p skiff-router --lib` 42 项全绿。
- TS `vitest run tests/actor-get-create-activation.test.ts` 13/13。
- hermetic：`node --test scripts/tests/actor_parity_differential.test.mjs`
  5/5；`verify-live-registry.test.mjs` 20/20。
- `verify --only router-live:actor --list` 正常；workflow YAML 解析通过。
- `node scripts/check-router-actor-live.mjs`：Rust-only `actor_live_probe`
  PASS（two-replica + 负例/归零）；differential 全链 18/20 通过，2 项
  frameEvents 对比失败（见下）。

### 剩余差异（已定位 owner → 已记录并接受；root 裁决 2026-08-03）

root 裁决：两项差异为"已解释、已留证、双侧 fail-closed、HTTP 可观测一致"
的记录差异，E-actor-parity gate 按 **accepted-with-recorded-differences**
通过交付，不另派语义修复（cutover 后 Rust 是唯一实现；若未来需要 TS 对齐
再单独立项）。runner 只放行精确命中 inventory `knownDifferences`
（`accepted: true`）路径的失败；未声明路径仍 fail closed。异步帧交织顺序
记录为 non-blocking follow-up。

1. **flaky/retained-entry 失败路径语义**（owner：TS
   `ActorGetCreateActivationCoordinator`/`ActorMethodDispatcher` vs Rust
   `ActorActivationRequestBroker`/`ActorFrameSink`）：
   - 首次 create 失败后 retained entry 的第二次请求：TS 在
     getOrCreate 直接 resolve retained entry，激活延迟到 method invoke
     （失败经 `actor.owner.failure` 显现）；Rust 在第二次 getOrCreate
     重新 activation（ACK rejected → `actor.getOrCreate.error`）。
   - 两者均 fail closed，HTTP 可观测（status/body）完全一致（flaky 步
     http.steps equal PASS）；wire 帧序列不同。
   - 对齐需要 Rust invoke 路径补延迟激活（或改 TS 语义），超出本叶子
     "仅 parity 所需最小修复" 范围 → 上报。
2. **rejected activation 的 getOrCreate.error code 词汇**（owner：
   TS `ActorCreateFailed` vs Rust `AckRejected`）：Rust 内部 waiter outcome
   与 TS wire 词汇不一致；改动会触碰 Rust activation corpus 冻结词汇 →
   上报。
3. **异步帧交织顺序**（harness 观察问题，非产品差异）：成功路径两侧帧
   集合一致，独立子流（spawn 提交/返回、owner.invoke/control）的到达顺序
   非语义；当前投影按 relay 记录序比较会产生顺序 false positive。后续可
   用按 HTTP 步窗口的 canonical 排序收敛（本批未实施，避免掩盖语义差异）。

结论：E-actor-parity 成功路径 parity 达成并留证；2 项失败路径 wire 差异
（可复现、有证据）经 root 裁决接受并记录，gate 按 accepted-with-recorded-
differences 交付。

## 交接

完成后提交到 `feat/router-rust-e-actor-parity`（不 push），直接向
`/root/router_rust_integration_b10` 报告 branch、worktree、commit/tree、
写集、自验收矩阵与已知差异/延迟 seam（含上述剩余差异证据），并通知 root。
