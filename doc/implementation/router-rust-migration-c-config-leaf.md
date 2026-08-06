# Router Rust Migration Batch 1 — C-config Leaf

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_c_config`
集成目标：`/root/router_rust_integration_b1`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-1.md`（在 `integration/router-rust-migration-batch-1` 上，基线 main 分支尚未包含；本叶子按路径引用，集成合流后可用）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，重点 §2.5 C0 / C-config；冲突时以权威设计为准）。
- 仓库约定：`AGENTS.md`（skiff repo）、`/Users/geek/workspace/AGENTS.md`（workspace，git 外）。
- Baseline：`main@9e492fa7`（`git rev-parse main` = `9e492fa77bb5129a5d872f964959449e929c2051`，已验证）。

## 预检结论（零 worktree 只读）

### Router process config 解析入口

任务描述写“当前 TS parser 在 `router/src/config/index.ts`”，预检确认实际入口不同：

- `router/src/router/config.ts`（608 行）是唯一的 Router **process** config parser：`loadRouterConfig(configPath, overrides)` 读取 `router.yml`，生产入口为 `router/src/router/server.ts`，测试入口为 `router/tests/config.test.ts`、`router/tests/router-bootstrap-session.test.ts`。
- `router/src/config/index.ts` 是 **service** config shape 机制（`skiff-config-shape-v1`、`config.yml` + `config.<profile>.yml` + secret 文件、redaction），不是 Router process config parser。本叶子只为其导出严格 YAML 解析共享 helper，不改变其语义。
- 本叶子按写边界将 `router/src/router/config.ts` 视为 “config 相关” 必需文件；否则 C-config 任务不可执行。

### 全部 renderer / consumer（写集依据）

Parser/renderer 契约两侧：

| 角色 | 文件 | 说明 |
| --- | --- | --- |
| Parser | `router/src/router/config.ts` | 冻结 schema/defaults/relative-path/redaction/unknown-key；本叶子唯一写入方 |
| Parser consumer | `router/src/router/server.ts` | 只读，不改 |
| Renderer 库 | `scripts/lib/runtime-stack-config.mjs` | `renderRouterConfig` 唯一 renderer 实现 |
| Renderer 调用方 | `scripts/skiff-instance.mjs` `routerConfigText`（约 480-491） | 只改该函数 |
| Renderer 调用方 | `scripts/skiff.mjs` `routerDevConfig`（约 731-743） | 移除 `ecosystemStoreCliPath` 传参 |
| Renderer 调用方 | `scripts/deploy-runtime-stack.mjs` `writeRouterConfig`（约 351-366） | 移除 `ecosystemStoreCliPath` 传参 |
| Instance URL 模型 | `scripts/lib/local-instance-config.mjs` | `urls.routerReload` / `instanceSummary.routerReloadUrl` |
| Instance URL 消费 | `scripts/skiff-instance.mjs` 202、1101-1103 | 仅 `urls.routerReload` 行 |
| Instance URL 断言 | `scripts/check-local-instance.mjs` 56、134 | 删除 reload URL 断言 |

### 与兄弟节点 ownership 无冲突

- C0-control 独占 `router/src/router/controlPlane.ts`、`httpGateway.ts` stale handler、`router/tests/artifact-reload.test.ts`、loop-risk evaluator/self-test/live baseline；本叶子不写。
- PR 0a 独占 `scripts/skiff-instance.mjs` process spawn/match（约 580-600、1350-1360）与 `router.implementation`/`RouterProcessSpec` 解析；本叶子只改 202、480-491、1101-1103。
- `verify-live-registry.mjs` 无 local-instance 条目（local-instance 在 `verify-checkers.mjs`，无需改动）。
- 无稳定 instance / Mongo / PM2 / 4004-4007 进程操作；`dev-home/router.yml` 为 git 忽略的本地同步。

### reload URL 现状表面（全部移除/更新）

| 表面 | 位置 | 处理 |
| --- | --- | --- |
| instance summary/status | `scripts/skiff-instance.mjs` 202、1101-1103 | 删除 `routerReload(Url)` |
| instance URL 模型 | `scripts/lib/local-instance-config.mjs` 146、213 | 删除 `routerReloadUrl`/`routerReload` |
| local-instance 断言 | `scripts/check-local-instance.mjs` 56、134 | 删除 |
| tooling README | `scripts/README.md` 43-47 | 改为 canonical `/__skiff/activate-assembly` 契约 |
| repo AGENTS | `skiff/AGENTS.md` “更新 artifacts 或 release pointer…” 段 | 改为 canonical activation 契约；历史 implementation record 保留 |
| architecture doc | `doc/architecture/test-runner-runtime-isolation.md` 33、121-133、169 | 更新为当前 activation URL 契约 |
| workspace AGENTS（git 外） | `/Users/geek/workspace/AGENTS.md` 端口表 4001、watch/reload 段 | 本地修改，不提交不 push |

## 冻结契约（本叶子实现）

### Schema（唯一 Router process config）

顶层键严格为：`activation`、`artifactsPath`、`devReload`、`environment`、`fileBackend`、`host`、`http`、`httpPort`（顶层别名）、`manifest`（单数别名）、`manifests`、`profile`、`releaseMode`、`requestTimeoutMs`、`rewrite`、`runtime`、`runtimePath`（顶层别名）、`runtimePort`（顶层别名）、`serviceDb`、`telemetry`、`websocket`。

嵌套：

- `activation.prepareTimeoutMs`（positive int，default 120000）
- `http.port`（default 4000）、`http.maxRequestBytes`（required）、`http.maxResponseBytes`（required）
- `runtime.port`（default 4001）、`runtime.path`（default `/runtime`）、`runtime.maxConcurrency`（required）
- `websocket.path`（default `/ws`）
- `serviceDb.mongoUrl`（required）
- `telemetry.enabled` / `endpoint` / `protocol`（`skiff-telemetry-v1`）/ `topics`（默认全量）/ `queueMaxEvents`（10000）/ `batchMaxEvents`（200）/ `batchMaxBytes`（262144）/ `flushIntervalMs`（1000）
- `fileBackend.local.root`；`fileBackend.oss.endpoint`、`bucket`、`region`、`accessKeyId`、`accessKeySecret`、`accessKeyIdEnv`、`accessKeySecretEnv`
- `rewrite[]`：`host`（required）、`path`（可选，`/` 开头）、`service`（required publication id）、`version`（可选）

顶层必填：`profile`、`artifactsPath`、`serviceDb.mongoUrl`、`http.maxRequestBytes`、`http.maxResponseBytes`、`runtime.maxConcurrency`。

其他默认：`host=127.0.0.1`、`requestTimeoutMs=20000`、`manifests=['fixtures/hello/manifest.json']`；`environment`/`devReload`/`releaseMode`/`telemetry`/`fileBackend` 缺省时按现状缺席。

### Relative-path resolution

相对 `router.yml` 所在目录解析：`artifactsPath`、`manifests`/`manifest` 每项、`fileBackend.local.root`。其余字符串是值不是路径。

### Unknown-key policy（本叶子引入 hard cut）

任何未声明键（顶层与全部嵌套对象）fail closed：`router config <dotted-path> is not supported`。已退役字段保留精确错误（`artifactRoot`/`artifactRoots`/`artifacts`、`hosts`、`values`、`http.bodyLimitBytes`、`serviceDb.storageNamespace`）。YAML duplicate key、anchor、alias、tag 一律拒绝；键名必须匹配 `[A-Za-z_][A-Za-z0-9_-]*` 且不含 `.`（与 service config 解析一致）。

### Secret redaction

新增 `redactRouterConfig(config)`：`serviceDb.mongoUrl`、`fileBackend.oss.accessKeyId`、`fileBackend.oss.accessKeySecret` 的直接值替换为 `[REDACTED]`（`ROUTER_CONFIG_REDACTED_VALUE`）。env 引用名不 redact。bootstrap 投影（`runtimeBootstrapForRouterConfig`）仍使用未 redact 值，因为 runtime 需要真实 Mongo URL；redaction 只用于诊断投影。

### Golden corpus

`router/tests/fixtures/router-config/`：

- `corpus.json`（`skiff-router-config-corpus-v1`）：valid/invalid 索引，invalid 每项带 expected error regex；TS vitest 消费，未来 Rust 消费同一 JSON/文件。
- `valid/*.yml`：canonical、minimal、defaults、aliases、telemetry、file-backend、direct-secrets、rewrite、manifests。
- `invalid/*.yml`：约 40 个负例（缺必填、类型/范围、unknown top-level/nested、legacy 字段、YAML 语法/duplicate/alias/anchor/tag、telemetry/rewrite/fileBackend 细节）。

刻意不放进 `cross-system-fixtures/`：该目录要求每 case 至少两个不同 system（compiler/runtime/router），本 corpus 的 TS 与未来 Rust 消费者都属于 `router` system。

## 写集

1. `router/src/config/index.ts`：导出 `parseStrictYamlObject`（现 `parseConfigYamlSource` 行为不变）。
2. `router/src/router/config.ts`：严格 schema + unknown-key 拒绝 + duplicate/alias/anchor/tag 拒绝 + `redactRouterConfig`。
3. `router/tests/fixtures/router-config/*`：golden corpus。
4. `router/tests/config-corpus.test.ts`：corpus 消费测试（valid resolve、invalid reject regex）。
5. `router/tests/config.test.ts`：unknown-key、duplicate key、YAML feature、redaction 用例。
6. `scripts/lib/runtime-stack-config.mjs`：移除 `ecosystemStoreCliPath`（输入显式拒绝），按 parser 契约补 renderer 校验（profile/host/environment/port/runtimePath/requestTimeoutMs/booleans/telemetryEndpoint/rewrite）。
7. `scripts/skiff-instance.mjs`：202、480-491、1101-1103。
8. `scripts/skiff.mjs`、`scripts/deploy-runtime-stack.mjs`：renderer 调用移除 `ecosystemStoreCliPath`。
9. `scripts/lib/local-instance-config.mjs`、`scripts/check-local-instance.mjs`：reload URL 表面删除。
10. `scripts/tests/runtime-stack-config.test.mjs`、`scripts/tests/skiff-instance-config.test.mjs`、`scripts/tests/runtime-stack-deploy.test.mjs`：同步。
11. `scripts/README.md`、`skiff/AGENTS.md`、`doc/architecture/test-runner-runtime-isolation.md`：reload URL 契约更新。
12. `router/router.example.yml`：冻结 schema 注释（内容已符合）。
13. `.skiff-instance/dev-home/router.yml`（git 忽略）：本地同步删除 `ecosystemStoreCliPath`，不动 instance。
14. `/Users/geek/workspace/AGENTS.md`（git 外）：本地同步，不提交。

## 自验收矩阵

| 项 | 命令 |
| --- | --- |
| router type-check | `pnpm --dir router run type-check` |
| router tests（含 corpus） | `pnpm --dir router test` |
| local-instance checker | `node scripts/check-local-instance.mjs` |
| scripts 聚焦 tests | `node --test scripts/tests/runtime-stack-config.test.mjs scripts/tests/skiff-instance-config.test.mjs scripts/tests/runtime-stack-deploy.test.mjs` |
| renderer 不再输出 `ecosystemStoreCliPath` | `rg -n ecosystemStoreCliPath scripts/lib/runtime-stack-config.mjs scripts/skiff-instance.mjs scripts/skiff.mjs scripts/deploy-runtime-stack.mjs`（无） |
| tooling/docs 无 reload-artifacts 现状契约 | `rg -n reload-artifacts scripts/README.md scripts/check-local-instance.mjs scripts/lib/local-instance-config.mjs skiff/AGENTS.md AGENTS.md`（无；历史 implementation record 与 C0 独占 router 文件除外） |

不跑全量 `pnpm verify`，不重启/修改 stable instance，不动 Mongo/PM2/4004-4007。
