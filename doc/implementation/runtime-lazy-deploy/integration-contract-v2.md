# Runtime Lazy-Load Deployment — 集成契约 v2（M3 公共面）

状态：开发总监钉死的缝点契约，M3a/M3b/M2b 共同事实源。补充
`runtime-lazy-load-deployment.md`（架构）与 `implementation-plan.md`（计划）的执行细节，
**不改变设计语义**。与架构文档冲突以架构文档为准，冲突时上报总监。

## 0. 已落地契约（M1/M2 合流，不再讨论）

- buildId = `ServiceDeploymentRef.deployment_artifact_identity` 字符串。
- Release 指针：`read_release_pointer(profile, service_id, version) -> StorageResult<Option<ReleasePointer>>`
  （deployment/src/storage/pointers.rs；`Some(ref)` 含完整 ServiceDeploymentRef；`None` = 未 set；
  Err = 记录缺失/非法，调用方视为不可解析 fail-closed）。
- request.start routing 段：`buildId: Option<String>`（4 个变体已有；runtime 消费，旧帧字节级兼容）。
- capabilities metadata：`artifactRoot: Option<String>` / `lazyLoad: bool` / `loadedBuildIds: Vec<String>`
  （空数组 = 未加载任何）。
- runtime 失败语义：加载失败（记录缺失/目录不可达/内容非法）→ 请求快速失败，不注册不缓存。

## 1. 候选投影规则（M3b 实现，唯一权威）

候选查询输入 = `CandidateQuery { mode, build_id }`（从 request.start header 的
`routing.build_id` 取；M3a 未合流时该字段 = deployment.identity，值等价）。

会话注册数据（M3b 扩展 SessionRecord / 能力存储）：
- `registeredBuildIds: Vec<String>`（来自 capabilities.loadedBuildIds，每次刷新覆盖）
- `lazyLoad: bool`、`artifactRoot: Option<String>`（来自 capabilities）
- 既有字段保留：registered / registration_revision / cancelled / dispatch capabilities

投影规则（逐 session，全部满足才候选）：
1. registered 且未 cancelled；
2. registration_revision 匹配；
3. `registeredBuildIds.contains(build_id)` **或**（`lazyLoad && artifactRoot == routerArtifactRoot`）；
4. dispatch capabilities 支持请求 mode（既有 `supports(mode)` 规则）。

routerArtifactRoot：由 M3b 从 bootstrap 打开的 CanonicalArtifactStore root 获取并注入候选查询源。
无候选 → 既有 `NoCandidate`（503 "no eligible runtime"）语义，不新增错误类。

## 2. HTTP 解析面（M3a 实现）

- 新模块 `router/src/release/`（或 http 内）：`ReleaseResolver` —— 输入
  `(service_id, version)`，输出 `Option<ServiceDeploymentRef>`（内部 `read_release_pointer`，
  Err/None 均 fail-closed）。profile 来源：router 既有 profile 配置/epoch。
- `HttpIngressBinding` 增加 `build_id`；`HttpIngressResolver` 不再依赖 epoch 的
  `gateway_ingress`/`deployment_projection`，改为从 deployment 记录重建 surface
  （复用既有 `HttpGatewaySurfaceView` 从记录重建的路径，`http/ingress.rs:64-96/321-346`）。
- request.start 帧构造（http/frame.rs）：buildId 用解析结果的 ref identity（替代现适配补丁
  从 binding.deployment 取的等价值）。
- error 语义：release 不存在 → fail-closed，可复用 404 `AssemblyIngressNotFound` 或新增
  `ReleaseNotFound`（M3a 自定，不得改变 503/504/502 既有映射）。
- **禁止触碰**：routing/、dispatch/、session/、supervisor/mod.rs（组装归 M3b）、health/。

## 3. 注册目录与组装（M3b 实现）

- SessionRecord / Directory：注册信息从 capabilities 帧更新（§1 字段），不再以 Register 帧
  tuple 为唯一身份（Register 帧保留至 M4）。
- `routing/query.rs`：`CandidateQuery` 改 `{ mode, build_id }`；`CandidateSession` /
  `RegisteredSessionLease` 携带 §1 字段；`project_session_with_capability` 按 §1 规则投影；
  `DeploymentNotInEpoch` 语义退役为"buildId 无候选"。
- `dispatch/candidate.rs`：`candidate_query_from_request` 从 header.routing.build_id 取；
  capabilities 解析扩展 lazy-load/artifactRoot/loadedBuildIds。
- `supervisor/session_ports.rs` + `supervisor/mod.rs`：组装全部由 M3b 负责（含 dispatcher
  端口、候选 view 源、routerArtifactRoot 注入）。
- `health/wire.rs`：activeAssembly 投影扩展（可展示 buildId 集合/能力者，M3b 自定形状）。
- **禁止触碰**：http/ingress.rs、http/server.rs、http/frame.rs、http/error.rs（M3a 拥有）。

## 4. 依赖闭包懒加载（M2b，M3 合流后排期）

- 加载入口 buildId 时，读其 deployment 记录的 `serviceSelectors`（contract 含
  serviceId+contractVersion）与 package 层 `service_requirements`，按
  `read_release_pointer(profile, service_id, version)` 递归解析 provider deployment，
  整闭包合成一个 image（provider 与调用方同 assembly，linker binding 自然成立）。
- 防环（已解析集合）；profile 来自 bootstrap 帧。
- M2 对 serviceSelectors 的 fail-closed 分支由本任务解除。
- 写面：runtime/loader/src/deployment.rs、runtime/host（loaded_deployments 相关）。

## 5. 里程碑提交点与验证

- M3a/M3b 各：核心改造提交一次，收尾提交一次；提交前 `git status` 核对写界。
- 每轮聚焦验证（秒~分钟）：`cargo check -p skiff-router`；单测试
  `cargo test -p skiff-router --no-fail-fast <filter>`（按文件/场景）。
- 里程碑收尾：`cargo test -p skiff-router --no-fail-fast` 全量（~3-8min）。
- 墙钟预算：每子里程碑 75 分钟硬上限，到点提交 + 四段式返回部分交付，由总监续派。
- M3b 收尾额外：`cargo check -p skiff-runtime-host`（消费面编译检查，禁入面只编译不跑）。
