# Runtime Lazy-Load Deployment — M4 契约（缝点契约 v3）

状态：开发总监钉死的 M4 共享契约，M4 全部并行 agent 的共同事实源。补充
`runtime-lazy-load-deployment.md`（架构）与 `implementation-plan.md`（计划）的 M4 执行细节，
**不改变设计语义**。与架构文档冲突以架构文档为准，冲突时上报总监。

已拍板决策（2026-08-06，总监确认）：

- `assembly.activation` 帧族（Prepare/Prepared/Reject/Commit/Abort/Register）**完全退役**。
- **无存量数据，不做世代迁移**；`assembly sync-state` 直接退役，不新增 migrate 命令。
- 合入 main 后 M5 在 stable 实例上验证（本契约只管 M4）。

## 0. 目标语义（M4 完成后）

- router 唯一可变部署状态 = release 指针表 `(profile, serviceId, version) → buildId`（文件，
  typed pointer store，M1 产物）。
- router bootstrap = 打开 CanonicalArtifactStore + profile 校验，**不加载任何全量部署状态**；
  deployment 一律按需经 `ReleaseResolver`（`read_release_pointer`）解析。
- runtime 注册 = capabilities-only（`loadedBuildIds` / `lazyLoad` / `artifactRoot`），无 tuple、
  无世代、无 Register 帧。
- **router 不再连接 Mongo**（activation repository 下线；`serviceDb.mongoUrl` 配置保留，经
  bootstrap 帧 `service_db` 传给 runtime）。
- 会话注册门 = profile + 能力通告；`NewGenerationBeforeEpochSwap` / `StaleRegister` 等 tuple
  终态退役。

## 1. Wire 契约（所有帧改动）

| 帧 | 处置 | 新形状 | 拥有者 |
| --- | --- | --- | --- |
| `assembly.activation`（全族） | **删除**：transport 定义 + codec + frame family 注册 + sink + 全部语料（activation-transaction-cases/raw-cases、w_model_activation_*、testdata） | 无 | R（transport）+ B（host 消费面） |
| `router.bootstrap` 的 activation 子头 | 简化 | `{ profile }`；删除 generation/assembly/config_snapshot。artifacts_path/service_db/http 子头不动 | 形状：R（transport codec）；消费：B（host `decode_connection_bootstrap`） |
| `request.start` 4 个 routing 头 | 形状冻结，tuple 字段变 Option 且不再填充/消费 | `buildId` 为唯一有效维度；assembly_identity/generation 保留字段但 router 不填、runtime 不 pin | R（transport + router）+ B（host 消费） |
| `websocket.generation.lifecycle` | 键控改 buildId | `assembly_generation: u64` → `build_id: String`（键控 per-connection 钉住 buildId） | R（transport）+ B（host 生成值） |
| `actor.owner.invoke/control` authority | 键控改 buildId | `assembly_identity/assembly_generation` → `build_id`（deployment 锚定，无 generation） | R（transport）+ B（host 消费） |
| `runtime.registered` ACK | 保留（握手完成帧） | 不变 | — |
| `runtime.capabilities` | 保留，成为唯一注册通告 | 不变（M3b 已含 lazy-load 面） | — |

corpus 更新责任：transport 侧语料（`runtime/transport/testdata/**`、`runtime/transport/tests/**`、
`runtime/tests/w_model_*`）归 R；host 侧语料归 B；`cross-system-fixtures/` 按消费方拆分（deployment
消费的归 A，transport/router 消费的归 R）。

## 2. Router 语义替换表（epoch 读点全量迁移）

| 读点（现状） | M4 后 |
| --- | --- |
| `activation/` 全模块（coordinator/repository/memory/http/recovery/retry/health/error/index） | 删除（R） |
| bootstrap reader/runner/assembly 读 committed 状态 + Mongo connect | 重写：打开 store + profile 校验；无 committed/pending 矩阵；fail-closed 只剩 store 打开失败/profile 非法 |
| `bootstrap/epoch.rs` `RoutingEpoch` / `ActiveRoutingEpochStore` | 删除；`actor_catalog` 改为按 deployment/buildId 从 actor routing projection 按需读取/缓存 |
| dispatcher `Pending.epoch` / admission 门 | 删除；门 = release 解析成功 |
| task image source / admission 成员资格 | `EpochTaskExecutionImageSource` → release 指针解析（profile, serviceId, version）→ deployment record → buildId |
| http/ws surface 视图（epoch.deployment_projection） | 指针表扫描重建（读全部 release 指针 → deployment records） |
| actor route authority / idleEvict | buildId 锚定 |
| session layer/handshake/directory tuple 校验 | 能力通告 + profile；tuple 终态删除 |
| health `activeAssembly` | 指针表投影：`{ profile, releaseCount, buildIds }` |
| health `activeRoutingEpoch` / `pendingActivation` | 删除 |
| config `activation.prepareTimeoutMs` | 删除（含 validate/render/redact/corpus） |
| listener `/__skiff/activate-assembly` 挂载 | 删除 |

## 3. Runtime 语义替换表（B 拥有）

| 面（现状） | M4 后 |
| --- | --- |
| `recover_durable_committed` / host recovery.rs | 删除（无 committed 恢复）；bootstrap 只设置 artifact root + profile |
| `SessionActivationState`（host/router_session/activation.rs） | 删除（Prepare/Commit/Abort 会话状态机） |
| `provisioning.rs` Prepare/Commit/Abort/Register 处理 | 删除；Register 帧不再发出 |
| `queue_connection_registration` | 只发 capabilities |
| `loaded_deployments` 双 image（or_insert） | 唯一加载源 = 懒加载闭包；注释与断言同步简化 |
| `apply_bootstrapped_assembly_activation_control` 等 | 删除 |
| `runtime/activation` ActivationId 派生 | tuple 缺省时由 buildId + deployment 派生 |
| host 的 assembly_identity/generation pin（gateway_ingress_pin、actor route hold、websocket_generation） | 改 buildId 键控/容忍缺省 |
| `runtime.registered` 接收面 | 保留（握手 ACK） |

## 4. test-runner（D 拥有）

- 删除 `--activation-url` / `--expected-generation` 及 validate；`activation_request_body`、
  activation receipt 解码、readiness 的 committed tuple / pendingActivation 校验全部退役。
- 每 batch：publish（authoring 已同事务写 release 指针，M1）→ readiness 检查：轮询
  `/__router/health` 至 `activeAssembly.buildIds` 含目标 buildId（或等价的指针可解析），
  首请求成功为最终门。
- `package_service_smoke_fixture` seed 不再写 `profiles/<profile>/activation.json`（删除
  `initialize_profile_activation` 调用）。

## 5. watch / CLI（C 拥有）

- `activateDevAssembly`（dev-assembly-activation.mjs）整体退役：dev-sync 发布已含指针写
  （M1 authoring 同事务），激活步骤删除即可；幂等 = publish 幂等。
- `skiff assembly activate` / `assembly sync-state` 退役；`assembly-state-sync.mjs`、
  `dev-assembly-activation.mjs`、`activation-timeout.mjs` 删除。
- `stack init` / isolated-test-runtime 的 Mongo `activation_state` 写入全部删除（seed 语义
  变为"空指针表基线"）。
- live registry：`router-rust-activation-full-chain-live` 退役；router-live:* 的
  `ActivationStateMongoHarness` seed 改为"指针表 seed（写 artifact store 文件）"或直接退役；
  `--activation-url`/`--expected-generation` 参数链（verify-live-plan/catalog）整链删除。
- `telemetry` TS 侧：`activationIdentity` 若值语义随 buildId 派生变化，同步测试。

## 6. 所有权表（写集不相交，单 worktree `skiff-integration`）

| Agent | 写集（文件域） | 自证（轮内） |
| --- | --- | --- |
| A deployment-crate | `deployment/src/activation_state/**` 删、`deployment/src/storage/activation.rs`(+tests) 删、storage/mod.rs 与 lib.rs 清理、`deployment/tests/{activation_reducer_contract,activation_state_contract,bootstrap_chain_corpus}.rs` 删、`artifact-identity` 的 `ProfileActivationStatePath`(+tests) 删、deployment 消费的 cross-system-fixtures（activation-state*.json、activation-request.json）删 | `cargo check -p skiff-deployment -p skiff-artifact-identity` |
| B runtime-host | `runtime/host/**`、`runtime/activation/**`（含全部相关测试） | `cargo check -p skiff-runtime-host` |
| C tooling-cli | `scripts/**`、`telemetry/**`（TS）、`scripts/tests/**`、`scripts/fixtures/**` | `pnpm --dir scripts test`；`pnpm --filter @skiff/telemetry test` |
| D test-runner | `test-runner/**`（含全部相关测试） | `cargo check -p skiff-test-runner`（若因 A 的删除中断，以收敛后为准） |
| R router-convergence | `router/**` 全部、`runtime/transport/**`、`runtime/tests/**`（w_model 等 transport 语料消费）、transport 消费的 cross-system-fixtures | `cargo check -p skiff-router`（以收敛后为准） |

跨 crate 缝：bootstrap 帧形状（§1）、request.start tuple 缺省、ws lifecycle / actor authority
的 buildId 键控、health 形状（§2）。**轮内 workspace 允许编译断裂**，收敛在 Round 3。

## 7. 纪律与验证门

- 不并发跑 cargo（共享 target 锁会排队）、禁止 `cargo clean`、不碰 `.stack/` 与 stable 实例
  （4000/4001/4002）、不提交 main、不修改所有权表外文件。
- agent 自证只跑写集内 crate 的 check/test；跨 crate 编译与全量验证收敛后统一做。
- Round 3（集成方）验证门：整仓 `cargo check` → `cargo test -p skiff-router --no-fail-fast`
  （82 集）、`-p skiff-runtime-host`（432+）、`-p skiff-runtime-transport`、`-p skiff-deployment`、
  `-p skiff-test-runner`、`pnpm --dir scripts test`、`pnpm --filter @skiff/telemetry test`、
  rustfmt/clippy gates。
- 测试退役原则：随被测物退役（coordinator/repository/recovery/activation 帧族/readiness
  receipt 等直接删）；语义仍成立的（dispatch/session/ws/actor 的 buildId 投影）改写保留。
