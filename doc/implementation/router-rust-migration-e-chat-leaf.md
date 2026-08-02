# Router Rust Migration Batch 10 — E-chat leaf（router-live:chat 本地等价）

日期：2026-08-03
状态：execution leaf（一次性有界会话）→ **TASK_SCOPE_EXPANDED（第二轮）**

第一轮阻塞（HTTP surface 重复 gateway entry key）已按 root 授权修复并通过自验收；
本地等价 gate 仍被第二个生产缺口阻塞（Runtime 注册后 ~1.35s 被 Router 关闭），
详见「阻塞项与证据」。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-10.md`（E-chat 节点）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5）
  §7 E-chat、§8 named integration tasks 的 `router-chat-full-chain-live` /
  `router-live:chat` 行与 trusted workflow 归属说明。
- 基线：`origin/main@edc111f8`（fetch origin 后锚定；本 worktree HEAD 即基线）。
- 既有 harness 模式：`scripts/check-router-bootstrap-live.mjs`、
  `scripts/check-router-ws-live.mjs`、`scripts/check-router-actor-live.mjs`
  （真实 compiler artifact + 临时 Mongo + 真实 Rust Router/Runtime binary +
  committed activation state 直种 + 真实客户端链路）。

## 任务

在 `/Users/geek/workspace/internals/agine` 的 `npm run e2e:chat-smoke` 上建立
`router-live:chat` 的本地等价 gate：

1. 固定 service artifact manifest：记录 Skiff commit SHA、internals commit SHA、
   skiff-packages commit SHA 与全部 service artifact identities；schema 与计划
   §8 的 pinned commits + artifact manifest 语义一致。
2. 用 isolated Rust Router 实例（真实 compiler artifact + 临时 Mongo +
   真实 Runtime + 真实 Rust Router binary）加载 manifest 中的 service
   artifacts，然后执行 chat smoke，并用
   `AGINE_E2E_INGRESS_HTTP_BASE` 指向该 isolated instance（smoke 机制已只读
   预检：默认 `http://agine.localhost:4003`，支持环境变量覆盖）。
3. 本地验证使用同一 manifest schema 与命令；记录证据（三个仓库 SHA +
   assembly/artifact identities + 结果）。
4. 若 chat smoke 需要修改 internals 代码才能指向 Rust 实例：停下上报，不擅自
   改 internals。

## 写入边界

允许：

- `scripts/check-router-chat-live.mjs`（本地等价 harness）
- `scripts/lib/router-chat-live-manifest.mjs`（manifest schema/校验）
- `scripts/fixtures/router-chat-live/manifest.json`（PASS 运行证据 manifest）
- `scripts/lib/verify-live-registry.mjs`（append `router-live:chat` 条目）
- `scripts/tests/verify-live-registry.test.mjs`（同步 LIVE_SELECTORS 断言）
- `.github/workflows/router-rust-integration.yml`（CI 占位 job + classifier
  pattern，注明 trusted workflow 归属）
- 本文档（叶子任务 + 证据）

禁止：`router/src`、`runtime`、`deployment`、internals 代码、`AGENTS.md`、
scripts README、verify selector graph、`skiff-instance.mjs`。

## Manifest schema（`skiff-router-chat-live-manifest-v1`）

```json
{
  "schemaVersion": "skiff-router-chat-live-manifest-v1",
  "pinned": {
    "skiff": { "repository": "skiff", "commit": "<full SHA>" },
    "internals": { "repository": "internals", "commit": "<full SHA>" },
    "skiffPackages": { "repository": "skiff-packages", "commit": "<full SHA>" }
  },
  "environment": "router-live-chat",
  "generation": 1,
  "assembly": { "assemblyIdentity": "skiff-runtime-assembly-v3:sha256:<64>" },
  "configSnapshot": { "snapshotId": "skiff-runtime-config-snapshot-v1:<32>" },
  "services": [
    {
      "serviceId": "agine.ai/api",
      "contractVersion": "0.1.0",
      "deploymentRevision": "sha256-<64>",
      "deploymentArtifactIdentity": "skiff-deployment-artifact-v4:sha256:<64>",
      "implementationPackageBuildId": "skiff-package-build-v10:sha256:<64>"
    }
  ],
  "packages": [
    {
      "packageId": "agine.ai/llm-api",
      "packageVersion": "0.1.0",
      "packageBuildId": "skiff-package-build-v10:sha256:<64>",
      "packageLocalAbiIdentity": "<identity>"
    }
  ],
  "smoke": {
    "command": "npm run e2e:chat-smoke",
    "cwd": "<agine root>",
    "ingressBase": "http://127.0.0.1:<leased port>",
    "status": "PASS",
    "finishedAt": "<ISO 8601>"
  }
}
```

校验由 `scripts/lib/router-chat-live-manifest.mjs` 严格实现（unknown key
reject、字段 pattern 校验），本地运行与真实 CI 的 private workflow 共用同一
schema。真实 CI 的 private workflow 归 internals 仓库；本节点只交付本地等价。

## Harness 设计（`scripts/check-router-chat-live.mjs`）

1. 从 env/默认 workspace 路径解析 skiff / internals / skiff-packages /
   agine 根目录；读取三个仓库的 HEAD commit SHA（internals 工作树若 dirty，
   记录在叶子证据中，不改 internals）。
2. 临时根：`skiff-package-service-smoke-fixture --bootstrap-only` 播种
   `skiff.run/std` artifact；然后按依赖闭包发布：
   - packages：`internals/packages/{agent,llm-api,llm-providers}`、
     `skiff-packages/{http-session,track}`；
   - services：`internals/{agine,aihub,codex-relay}/service`。
   对每个 root 执行 `package publish`，缺 exact pointer 的依赖项延迟重试
   （同 `skiff-dev-sync.mjs` 的 buildDependencyOrdered 语义）。
3. 用 `rootDeployments` 投影 RuntimeAssembly；用 profile `dev` 生成 runtime
   config snapshot（服务 config.dev.yml / config.dev.secret.yml 均被读取）；
   写 `records/actor-routing/current.json`（A2 要求，内容同既有 gate）。
4. 组 manifest（三仓库 SHA + assembly + config snapshot + 全部 service
   deployment / implementation package / package artifact identities），
   校验 schema。
5. 临时 Mongo replica set（45000-45999 租约，`ActivationStateMongoHarness`）；
   `cargo build -p skiff-router --bin skiff-router` 与
   `cargo build -p runtime --bin runtime`（本 worktree 的
   `build/cargo-target`）。
6. 写 router.yml（`releaseMode: true`、`websocket.path: /ws`、
   `serviceDb.mongoUrl` 指向临时 Mongo）、runtime.yml（runtime-home +
   临时 service-db keyring）；mongosh 直种 committed activation state
   （environment `router-live-chat`、generation 1、assembly + snapshot），
   与既有 probe 同 schema（`{_id, revision, state}`）。
7. spawn 真实 Rust Router 与真实 Rust Runtime；等待 `/__router/health` 出现
   committed assembly identity + healthy replica（同 ws/actor probe）。
8. 在进程内启动 local ingress（复用 `scripts/local-ingress.mjs` 的
   `startLocalIngress`），把 `127.0.0.1:<ingressPort>` 映射到
   `agine.ai/api` 0.1.0。
9. 从 aihub `config.dev.secret.yml` 读取 deepseek apiKey（或
   `SKIFF_ROUTER_CHAT_LIVE_AIHUB_API_KEY` env），写成临时
   `service.aihub.apiKey` YAML（0600），供 smoke 的
   `AGINE_E2E_PROVIDER_SECRET_CONFIG` 使用；在 `internals/agine` 执行
   `npm run e2e:chat-smoke`，env 覆盖 `AGINE_E2E_INGRESS_HTTP_BASE`。
10. 要求 exit 0；写 evidence（manifest + smoke 日志摘要 + 四 SHA）；
    cleanup（SIGTERM 子进程 → ingress close → mongo cleanup → 端口断言 →
    temp 清理）。

## 自验收

- `node scripts/check-router-chat-live.mjs` → chat smoke PASS。
- `scripts/fixtures/router-chat-live/manifest.json` 含三仓库 SHA +
  assembly identity（四 SHA）+ 全部 service artifact identities + PASS 结果。
- `node scripts/verify.mjs --only router-live:chat --list` 可见任务；
  `scripts/tests/verify-live-registry.test.mjs` 全绿。

## 风险与停止条件

- internals/agine 或 service 构建/运行缺基础设施：停下上报。
- chat smoke 需要改 internals 代码：停下上报。
- 发现 Rust Router 生产缺口需动 `router/src`：停下上报（本节点禁止写）。

## 阻塞项与证据（2026-08-03 本地运行）

### 结论

#### 已修复：HTTP surface 重复 gateway entry key（root 授权，已完成）

真实 agine stack 中 `agine.ai/aihub` 与 `agine.ai/codex-relay` 都发布
`v1ModelsGet`（`GET /v1/models`）。按 canonical model（`ServiceIngressKey {
deployment, selector }`）把 HTTP surface 视图键改为
`(ServiceDeploymentRef, GatewayEntryKey)`：

- `router/src/http/ingress.rs`：`HttpGatewaySurfaceView` 以
  `(deployment, gatewayEntryKey)` 为键；`resolve()` 用
  `binding.deployment + gateway_entry_key` 查 surface；epoch 视图构建不再
  fail closed 于跨 deployment 重复 key。
- `router/src/supervisor/http.rs`：`load_http_surface_view` 同步（同键可跨
  deployment 合法存在，每个 deployment 记录内部 key 仍唯一）。
- 回归测试 `router/tests/ws_live_surface.rs`：
  `duplicate_gateway_entry_key_across_deployments_is_deployment_scoped` ——
  两个 deployment 同发 `v1ModelsGet`（GET /v1/models），surface 正常加载
  （len=2）且按 service selector 各自解析到正确 deployment；不匹配 path
  fail closed。
- 自验收：`cargo test -p skiff-router` 全绿（含既有 HTTP/WS/actor/differential
  suites，live probes 预期 ignored）；`ws_live_surface` 3/3。

#### 仍然阻塞：Runtime 首次注册后 ~1.35s 被 Router 关闭（新缺口，超出本轮授权）

修复第一缺口后，真实链路已跑通到 chat 执行前：session、provider list、
WebSocket、agents/create、chat/create 全部经 Rust Router + 真实 Runtime
成功；`npm run e2e:chat-smoke` 在 `chat/send` 处失败：

```text
[agine:e2e] chat ok id=chat_...
[agine:e2e] failed: ChatSmokePrimaryAndCleanupError: chat/send failed:
Runtime request runtime_disconnect: runtime_disconnect
```

证据链（KEEP_ON_FAILURE 保留实例 + RUST_LOG=debug 复现）：

- Router 对 Runtime 发送**空 Close 帧**（runtime debug log：
  `Received close frame: None`），Router stderr 无任何输出；Runtime
  随后 `runtime.router_disconnected` + 重连。
- 时序与 chat/send **无关**：每次全新 isolated instance 都是
  “Runtime 注册成功 → ~1.35s 后被关 → 重连再注册后稳定”。smoke 的
  chat/send 恰好落在这个窗口所以失败；等待 >2min 后手动执行
  session/provider/WS/agents/create/chat/create/chat/send 全部成功。
- 排除项：无 client WS 时 chat/send 同样杀会话；subagentsEnabled=false
  同样杀会话；deepseek API key 直连有效（200）；aihub 的
  `/v1/chat/completions` 经 Router 直调成功且会话存活；裸 WS 保持 6s
  正常；首次注册后的第二次注册稳定不再被杀。
- 另一独立现象：Runtime 冷启动时 whole-assembly service DB index
  provisioning 需 ~10-21s，超过 session handshake 的
  `bootstrap: 10s` 时限时第一次连接被关（随后重连复用缓存完成注册）。

疑似 owner（需 router 侧诊断日志定位，本叶子禁止写这些模块）：
`router/src/session/*`（handshake/ack barrier、`SessionTiming`）、
`router/src/activation/coordinator.rs`（cold recovery rebind 观察路径）、
或 `router/src/dispatch|ws` 的 abort/violation 路径。修复后重跑
`node scripts/check-router-chat-live.mjs` 即可继续。

### 运行证据（harness `scripts/check-router-chat-live.mjs`）

- 基线/仓库 SHA：
  - Skiff：`edc111f888a70743a8ecadc3bdbcb6b4ae2fd54a`（origin/main，worktree HEAD）
  - internals：`deff2357393d539013db843538b0d229f9bd5174`（HEAD；工作树有 6 个
    未提交改动，diff sha256 `2f95b53b75bc331a36cd111afc17cdb4525b0bb5357a4888b2d49b795b379f1e`，
    smoke 在未改动的 working tree 上运行）
  - skiff-packages：`db4ddd9e05936b6fa8beff42ed242c8a73f08de3`
- 真实 compiler artifact 构建成功：3 个 service deployment（agine.ai/api、
  agine.ai/aihub、agine.ai/codex-relay）+ 8 个 package artifacts。
- Assembly：`skiff-runtime-assembly-v3:sha256:2a8fa377f903002abcbbf972050f64f1fdef183741aebe956290b2e611e5ff3d`
- Config snapshot：`skiff-runtime-config-snapshot-v1:fca93462687b4b36a84014a45c061e95`
- 临时 Mongo replica set、真实 Rust Router/Runtime binary 均正常启动到
  “waiting for readiness” 阶段；Router 在 epoch 组合时 fail-stop（证据日志见
  上方错误行；完整输出保留在会话日志）。
- 三仓库 SHA + assembly identity 即任务要求的“四 SHA”；service deployment
  artifact identities 已按 `skiff-router-chat-live-manifest-v1` schema 校验通过
  （`scripts/lib/router-chat-live-manifest.mjs`，示例数据 services=3 /
  packages=8 验证 PASS）。
- 注意：第二轮起 Skiff SHA 为分支头 `f140ee8d…`（含 HTTP surface 修复）。

### 已交付（本分支）

- `scripts/check-router-chat-live.mjs`（isolated 实例 harness + `--preflight`）
- `scripts/lib/router-chat-live-manifest.mjs`（manifest schema 严格校验）
- `router/src/http/ingress.rs` + `router/src/supervisor/http.rs`（HTTP
  surface per-deployment 键修复，root 授权）
- `router/tests/ws_live_surface.rs`（重复 gateway entry key 回归测试 + API
  适配）
- `scripts/lib/verify-live-registry.mjs` 的 `router-live:chat` 条目
- `scripts/tests/verify-live-registry.test.mjs` LIVE_SELECTORS 同步
- `.github/workflows/router-rust-integration.yml` CI 占位 job（注明 trusted
  workflow 归属）
- 本叶子文档与阻塞证据

gate 未 PASS，`scripts/fixtures/router-chat-live/manifest.json` 未生成（harness
只在 smoke PASS 时写 evidence manifest）；集成 Agent 在第二个缺口修复后重跑
`node scripts/check-router-chat-live.mjs` 即可产出并提交该 fixture。

<!-- 运行结果由 harness 填充后回填 -->
