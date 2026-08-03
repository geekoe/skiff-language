# Router Rust Migration Batch 8 — W-differential Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_w_differential`
集成目标：`/root/router_rust_integration_b8`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-8.md`
  （W-differential 节点；baseline `origin/main@d228b613`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5）：
  §9 Continuous Integration Matrix（implementation-neutral differential
  harness：TS/Rust 独立端口、artifact root、runtime home、Mongo namespace，
  不共享 Runtime、不镜像 live traffic；对比 HTTP、WS、Runtime frames、
  health、Mongo state/audit、terminal counters；normalization 仅允许 UUID、
  timestamp、ephemeral port、无语义 log order）、§8 named tasks（显式
  live/manual selector，不进 default router selector）、§2.5（test ledger：
  retired / shared owner / Rust replacement / black-box replacement）、
  §5.1（RouterProcessSpec：supervisor、isolated runtime、differential
  harness、platform-source probe 共用同一解析）。
- 直接父批次交付：`doc/implementation/router-rust-migration-batch-7.md`
  （`router-live:session` managed harness、`verify-live-registry.mjs`
  live/manual 注册机制、45000-45999 租约约定）。

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`。
- 精确 baseline：`origin/main@d228b613eafeba5e2275bf830f5770f21b931e81`
  （worktree HEAD 已验证为同一 commit）。
- 分支 / worktree：`feat/router-rust-w-differential` /
  `/Users/geek/workspace/wt-w-differential`。
- 共享主 worktree 只读：本地 main（`40fac3b6`，未 push）落后 origin/main
  10 个 commit，且主 worktree 含未跟踪的 batch-8 调度文档；本节点一律以
  origin/main 为基线，禁止参考/合并/回退本地 main。

## 零 worktree 只读预检结论

1. **基线锚定**：`d228b613` 对象存在于共享仓库；`origin/main` 指向
   `d228b613eafeba5e2275bf830f5770f21b931e81`。
2. **isolated-test-runtime 机制**（`scripts/lib/isolated-test-runtime*.mjs`）：
   46000-46999 连续端口租约、temp root 所有权收据、
   `skiff-instance.mjs supervise` 起受管 Mongo；`isolatedTestInstanceConfigText`
   通过 `router.implementation: ts|rust` 选择 Router 进程。
   W-differential **不复用**该 supervisor（避免依赖 instance 装配），改为
   直接复用同一 `RouterProcessSpec` seam 与渲染器，自行编排隔离实例。
3. **既有 live harness 模式**（`check-router-bootstrap-live.mjs`、
   `check-router-session-live.mjs`，均在 d228b613）：真实 compiler
   authoring（`runCompilerAuthoring`/`runConfigSnapshotAuthoring`）→
   45000-45999 端口租约 → `ActivationStateMongoHarness` 临时 replica set →
   cargo 构建显式 Rust binary → 真实边界探针 → 确定性 cleanup
   （SIGTERM→SIGKILL、端口关闭断言、租约释放、temp root 删除）。
   W-differential 沿用同一资源模式，但驱动者改为 implementation-neutral
   Node 脚本（relay + HTTP + Mongo 查询），不新增 Rust probe test。
4. **verify 注册结构**：`verify-live-registry.mjs` 是 live/manual selector
   的唯一声明处；新增 `router-live:differential`（fixed-command、managed、
   live/manual）会出现在 `verify --list`/`--help`，且天然不进 default
   （`verify` 只展开 tests/rust-quality/type-check/checks；manual `router`
   只展开 router-ts-tests/router-rust/router-rust-process-smoke）。
   `verify-live-catalog.mjs` 要求新 `scripts/check-*.mjs` 恰好注册一次；
   `scripts/tests/verify-live-registry.test.mjs` 硬编码 `LIVE_SELECTORS`
   精确列表，注册时必须同步更新该断言（batch-7 加 `router-live:session`
   时同模式）。
5. **platform-source-probe**（`scripts/lib/platform-source-probe-*.mjs`）：
   与 differential harness 共享 §5.1 `RouterProcessSpec` 解析
   （`dev-runtime-paths.mjs`：`resolveRouterProcessSpec`/
   `assertRouterProcessSpec`/`routerProcessInvocation`）；differential
   进程启动只经该 seam，禁止分别硬编码 pnpm/tsx/binary 判断。
6. **Runtime frames wire**：`SKBF` 二进制帧
   （`runtime/transport/src/protocol/frame.rs`：magic + version + encoding +
   header len + payload len + JSON header + payload）；Node 可直接解码，
   frame type 取 header `type`。`session_live_probe.rs` 的 test-only WS
   relay（real Runtime ↔ relay ↔ real Router）是 frame 捕获的既有模式，
   W-differential 用 Node `ws`（经 router/package.json 解析，与
   loop-risk 一致）实现同构 relay。
7. **Mongo namespace 事实**：TS Router 使用 mongoUrl 路径指定 database，
   集合 `router_assembly_activation_states` /
   `router_assembly_activation_audit`；Rust binary 使用默认 database
   `skiff-router`，集合 `activation_state` / `activation_audit`；两侧文档
   形状均为 `{_id: environment, revision, state}`，可 seed 同一语义
   `EnvironmentActivationState`（generation 0/1、assembly ref、
   config snapshot ref）。
8. **基线可观测差异**：d228b613 的 Rust listener 对 public/control HTTP
   一律返回 200 空体（`/__router/health` JSON 未装配，归 W-composition/
   E-http）；TS 提供完整 health JSON。因此首个可跑场景只比较 HTTP status
   + Runtime frames + Mongo state/audit + terminal，health JSON 对比在
   inventory 中标记 `planned`（不虚构当前可跑断言）。
9. **禁止触碰清单**：router/src（Rust 生产代码）、router TS、
   runtime crate、runtime/transport/src、deployment、AGENTS.md、
   scripts README、verify selector graph、skiff-instance.mjs、CI workflow；
   不操作 stable instance/Mongo/PM2/4004-4007；不跑全量 `pnpm verify`。

## 任务目标（W-differential）

1. implementation-neutral differential harness（`scripts/` 下）：
   - TS/Rust 每侧独立端口块、artifact root、runtime home、Mongo namespace
     （独立临时 mongod + 独立 database/collection）；不共享 Runtime；
     不镜像 live traffic；
   - 场景驱动：scenario inventory 落盘（fixtures JSON + 文档），harness
     按 scenario 定义执行 capture → normalize → compare；
   - 观察类型覆盖：HTTP、WS（client WS 场景为 planned）、Runtime frames
     （relay 双向捕获）、health、Mongo state/audit、terminal counters；
   - normalization 仅允许 UUID、timestamp、ephemeral port、无语义 log
     order，且每个场景显式声明应用哪些 normalization 到哪些路径；场景
     配置字段（artifact root / mongoUrl / 端口 / runtime home）按
     side-expected 断言与各自配置精确一致，不属于 normalization；
   - 确定性 cleanup：SIGTERM → 等待退出 → 端口关闭断言 → lease 释放 →
     temp root 删除。
2. verify 注册：`verify-live-registry.mjs` 增加
   `router-live:differential`（fixed-command、managed、live/manual、
   requiredExecutables: node/pnpm/cargo/mongod/mongosh、
   requiredModules: ws from router/package.json）；同步更新
   `verify-live-registry.test.mjs` 的 `LIVE_SELECTORS` 断言；不修改
   ordinary selector graph，不进 default `verify`/`router`。
3. 场景 inventory 文档：`doc/implementation/...-differential-scenarios.md`
   （观察类型、normalization 政策、scenario 状态矩阵、按 §9 lane 的未来
   scenario）。
4. 测试 ledger：`doc/implementation/...-batch-8-test-ledger.md`，按 §9
   四类处置（retired / shared owner / Rust replacement / black-box
   replacement）记录；本节点不删除 TS test，ledger 记录 baseline 审计与
   删除协议，A2 的删除必须在本 ledger 登记。
5. 自验收：harness 对既有 TS/Rust 实例跑通至少一个场景（无业务断言也
   可）；`verify --list` 含 `router-live:differential`；`rg` 证明该
   selector 不在 default `verify`/`router` 展开中。

## 写入边界

可写：

- `scripts/lib/router-differential/`（harness 模块）。
- `scripts/check-router-differential-live.mjs`（verify/live 入口）。
- `scripts/fixtures/router-differential/`（fixture service source、
  scenario inventory JSON）。
- `scripts/lib/verify-live-registry.mjs`（仅新增 differential live entry）。
- `scripts/tests/verify-live-registry.test.mjs`（仅同步 `LIVE_SELECTORS`
  断言）。
- `scripts/tests/router-differential-*.test.mjs`（hermetic 单测）。
- `doc/implementation/router-rust-migration-batch-8-w-differential-leaf.md`、
  `doc/implementation/router-rust-migration-batch-8-differential-scenarios.md`、
  `doc/implementation/router-rust-migration-batch-8-test-ledger.md`。

禁止：

- `router/src/`、router TS（src/tests）、runtime crate、
  `runtime/transport/src`、`deployment/`；
- AGENTS.md、scripts README、`verify-selector-graph.mjs`、
  `skiff-instance.mjs`、CI workflow；
- 操作 stable instance / stable Mongo / PM2 / 4004-4007；不跑全量
  `pnpm verify`；不跑 chat smoke。

## 设计摘要

`scripts/check-router-differential-live.mjs`（verify 入口）与
`scripts/lib/router-differential/`（constants、frames、relay、mongo、
normalize、compare、instance、scenarios）构成 scenario 驱动的差分引擎：

1. authoring：fixture service（`scripts/fixtures/router-differential/ping/`）
   经真实 compiler 产出 RuntimeAssembly + config snapshot + actor-routing
   projection，复制到 TS/Rust 两个独立 artifact root；
2. 每侧独立资源：3 连端口（http/runtime/relay）+ 独立临时 mongod；
   RouterProcessSpec 按 `implementation: ts|rust` 解析并启动；real Runtime
   进程经 Node relay 连接各自 Router 的 `/runtime`；
3. capture：HTTP status、relay 双向 Runtime frames（SKBF decode +
   frame type）、health（当前 scenario 仅 status）、Mongo state/audit
   （按各实现 namespace 查询后 decode 语义文档）、router/runtime 日志、
   SIGTERM 后 exit code 与端口关闭状态；
4. compare：按 scenario spec 执行 `equal`（跨实现 deep-equal，先按声明
   normalization）/ `sideExpected`（与各侧配置精确一致）/ `recordOnly`
   （证据记录）三组断言，输出归一化 diff 报告；
5. scenario inventory：`scenario-inventory.json`（机器可读）+ 文档，首个
   runnable 场景 `session-handshake-basic`（HTTP status + session handshake
   frames + Mongo state/audit + terminal），后续 lane 场景标记 planned。

## 自验收矩阵

| 验收项 | 命令 / 证据 |
| --- | --- |
| harness 对既有 TS/Rust 实例跑通至少一个场景 | `node scripts/check-router-differential-live.mjs --scenario session-handshake-basic`（每侧 real Router + real Runtime + temp Mongo） |
| verify --list 含新条目 | `node scripts/verify.mjs --list`（或 `--only router-live:differential --list`）含 `live:router-rust-differential` |
| 不进 default | `rg -n 'router-live:differential' scripts/lib/verify-selector-graph.mjs scripts/lib/verify-plan.mjs` 零命中；`verify --only verify --list` 不含该 id |
| inventory 落盘 | `scripts/fixtures/router-differential/scenario-inventory.json` + 场景文档 |
| ledger 落盘 | batch-8 test ledger 文档（四类处置 + baseline 审计） |
| hermetic 单测 | `node --test scripts/tests/router-differential-*.test.mjs` |
| 写集干净 | `git status` 仅本 leaf 与上述可写文件 |

## 交接

完成后向 `/root/router_rust_integration_b8` 报告 branch、worktree、commit
hash、验收命令与结果；同步通知 root。

## 执行结果（2026-08-02）

状态：完成。

交付文件：

- `scripts/lib/router-differential/`：constants、frames（SKBF decode）、
  relay（test-only WS relay，real Runtime ↔ relay ↔ Router）、mongo（各
  实现 namespace seed/read/audit）、normalize（四类 normalization 白名单）、
  compare（equal/sideExpected/recordOnly + exclude 契约）、scenarios
  （inventory 校验）、instance（每侧隔离实例编排）、harness（顶层
  orchestrator）。
- `scripts/check-router-differential-live.mjs`：verify/live 入口
  （`--list`/`--scenario`/`--only`/`--keep-temp`/`--json`）。
- `scripts/fixtures/router-differential/`：ping fixture service +
  `scenario-inventory.json`（runnable 1 + planned 6，含 blockedOn）。
- `doc/implementation/router-rust-migration-batch-8-differential-scenarios.md`
  与 `...-test-ledger.md`（§9 四类处置 + d228b613 66 个 TS test 的 retained
  审计 + 删除登记协议）。
- `scripts/lib/verify-live-registry.mjs`：新增 `router-live:differential`
  （managed、live/manual、id `live:router-rust-differential`）。
- `scripts/tests/verify-live-registry.test.mjs`：同步 `LIVE_SELECTORS`
  断言。
- `scripts/tests/router-differential-*.test.mjs`：hermetic 单测
  （frames/normalize/scenarios/compare，15 项）。

实现要点：

- 每侧独立 3 连端口（http/runtime/relay，45000-45999 租约）+ 独立临时
  mongod + 独立 database/collection namespace + 独立 artifact root /
  devHome / runtime home + 独立 real Runtime 进程；Router 进程只经
  `RouterProcessSpec` seam 解析启动。
- artifact 每个 scenario author 一次（真实 compiler + config snapshot
  tooling），再复制到两侧独立 artifact root——独立 authoring 会把 artifact
  root 编入 config snapshot 记录导致 snapshotId 不一致（已实测验证并规避）。
- `session-handshake-basic`：比较 HTTP control health status、完整
  runtimeFrames 序列（bootstrap/capabilities/Register/registered/health，
  timestamp normalization 仅 applied 到 `observedAt`）、Mongo state/audit、
  terminal（router SIGTERM exit 0 / runtime SIGINT exit 0 / 端口关闭）；
  bootstrap 帧的 artifactsPath/mongoUrl 经 sideExpected 与各侧配置精确
  一致，并从整数组 equal 中显式 exclude（inventory 校验强制 exclude 必须
  被 sideExpected/recordOnly 覆盖）。
- 自验收：`session-handshake-basic` 连续 3 轮 PASS（23 项断言）；
  `verify --only router-live:differential --list` 含
  `live:router-rust-differential`；`rg` 证明该 selector 不在
  verify-selector-graph/verify-plan/checkers/rust-subjects 中；
  `verify --only verify --list` 与 `verify --only router --list` 均不含它；
  新增 harness 单测 15 项 + verify-live-registry 20 项全绿；
  `check-javascript-syntax` PASS；45000-45999 无残留监听。
