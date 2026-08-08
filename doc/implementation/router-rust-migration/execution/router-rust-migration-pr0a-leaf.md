# Router Rust Migration PR 0a Leaf Task

日期：2026-08-02

状态：execution leaf（一次性有界会话）

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-1.md`（PR 0a 节点）
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5）
  - §5.1：`router.implementation: ts | rust` + 唯一 `RouterProcessSpec` + empty
    `skiff-router` Cargo package + `routerBinary` dev path / build-install /
    process matching / binary SHA-256 identity + implementation-neutral smoke
    harness + `router-rust` subject + manual `router` 迁移期展开。
  - §6.2(1)：W-process/tooling 第一步（process selection、empty binary、dev
    path、process match、Rust consumer task）。
  - §7 PR0a：可选择的进程与无 listener skeleton，不实现业务 protocol。
  - §8：named tasks —— `router-rust` / `router-rust:contracts`
    （leaf `router-rust-contracts`）、`router-rust-process-smoke` /
    `router-rust:process-smoke`；manual `router` 展开规则与 graph transition
    去重覆盖。
  - §11.2：rollback manifest/schema/builder 与 TS/Rust process commands
    （本节点只做 schema + builder + 自测，不实现最终 immutable unit 重建）。
- 仓库：`/Users/geek/workspace/skiff`，baseline `main@9e492fa77bb5129a5d872f964959449e929c2051`
  （git rev-parse 已验证）。
- worktree：`/Users/geek/workspace/wt-pr0a`，branch `feat/router-rust-pr0a`。

## 任务边界

PR 0a 交付（详见批次文档与权威设计）：

1. instance config 严格迁移期字段 `router.implementation: ts | rust`；唯一
   `RouterProcessSpec { implementation, config_path, ts_source_root?, rust_binary_path? }`
   由 checkout + dev-home canonical resolver（`scripts/lib/dev-runtime-paths.mjs`）
   产生，不读 ambient env 决定 implementation。instance supervisor、isolated
   runtime harness、platform-source probe、process smoke harness 消费同一 spec，
   不各自判断 pnpm/tsx/binary。
2. 空 `skiff-router` Cargo package/binary：只支持 direct process
   identity/lifecycle smoke，不绑定 listener、不实现业务 protocol；提供 binary
   SHA-256 identity（`--identity` 自报 + 外部 harness 复核）。
3. `routerBinary` dev path、build/install placeholder、process matching 理解
   TS/Rust spec。`cargo-metadata` source key 不属于本节点。
4. verify 注册：Rust subject `router-rust`（leaf `router-rust-contracts`，task
   `router-rust:contracts`，subject 自动生成唯一 Cargo test leaf）；leaf task
   `router-rust:process-smoke`；manual `router` 迁移期展开
   `router-ts-tests` + `router-rust` + `router-rust-process-smoke`；
   implementation-tests / manual router / Rust subject 展开后 task 去重。
5. CI：`.github/workflows/verify.yml` PR job 增加 fast
   `router-rust` contracts/process smoke scope。不创建
   `router-rust-integration.yml`。
6. rollback manifest/schema/builder 与 TS/Rust process commands（§11.2 第一条）：
   新 builder 消费 `RouterProcessSpec`；TS unit 基于当前 TS source，Rust unit 用
   `rust_binary_path`；先做 schema + builder + 自测。

非目标：不解析旧 Router process config（C-config 冻结）、不写 AGENTS.md /
scripts README（C-config）、不写 control plane / loop-risk（C0-control）、不实现
listener / HTTP / WS / actor / Mongo、不改 `build-runtime-stack.mjs`
（cargo-metadata source key 后续节点）、不创建
`router-rust-integration.yml`（留给第一个 live slice）。

## 预检结论（main@9e492fa7）

- `scripts/skiff-instance.mjs`（1827 行）：`managedProcessSpecs()` 中 router
  literal 在 585-599 行附近；`commandMatchesComponent()` 的 router 分支在
  1352-1353 行，复用 `commandLooksLikePnpmDev` / `commandLooksLikeTsxService`；
  `loadInstance()` 调 `readInstanceConfig()`；`buildComponentBinaries()` 构建
  runtime/compiler。`routerConfigText` / `urls.routerReload` 行归 C-config，
  本节点不触碰。
- `scripts/lib/local-instance-config.mjs` 不识别 `router:` 顶层 key 也不拒绝
  unknown key（C-config owner）。PR 0a 在 `skiff-instance.mjs` 内用
  `simple-yaml` 严格解析 `router.implementation`，不写
  `local-instance-config.mjs`。
- `scripts/lib/dev-runtime-paths.mjs`（45 行）：`devRuntimePaths()` 返回
  devHome/artifactRoot/runtimeBinary/routerConfig 等；可扩展
  `routerBinary` + `resolveRouterProcessSpec` + `routerProcessInvocation`。
- verify 机制：`verify-selector-graph.mjs` 的 `publicSelectors` /
  `expansions`；`verify-rust-subjects.mjs` 的 subject registry +
  `assertRustWorkspaceOwnership`；`verify-plan.mjs` 的 leaf builders +
  `assertOrdinaryTaskBuilderCoverage`；`expandSelectors` 按 leaf 去重。
  `verify-taxonomy.test.mjs` / `verify-rust-quality.test.mjs` 断言 selector 展开
  与 CI commands，需同步更新。
- `checkerTasks('router-rust')` 返回空（无 checker 注册），Rust subject 只生成
  唯一 Cargo test task。
- `scripts/lib/isolated-test-runtime-instance.mjs`：
  `isolatedTestInstanceConfigText()` 生成 fixture config；消费点=显式写入
  `router.implementation`，其余仍由 `skiff instance supervise` 统一处理。
- `scripts/lib/platform-source-probe-node-dependencies.mjs`：
  `prepareOwnedRouterNodeDependencies()` 当前硬编码 `router` 目录与 tsx；
  消费点=从 `RouterProcessSpec.ts_source_root` 派生。
- `router/` 与 `router/src` 混放 `.rs` 无工具冲突：`runtime-execution-boundary`
  的 TS 扫描按 `\.(?:c|m)?tsx?$` 过滤；`tsconfig` include 仅 `**/*.ts`；
  vitest 默认仅匹配 test 文件；pnpm 忽略 Cargo.toml；cargo 忽略 package.json。
  Cargo workspace member 选择 `router`（与 TS package 并存，最终 cutover 后
  `router/src/main.rs` 即为 Rust router 的落点）。
- `.github/workflows/verify.yml`：单个 matrix job 三 scope；新增第四 scope。
- CI YAML 解析可用 `python3 -c 'import yaml'`。

## 设计契约

### RouterProcessSpec（JS 对象，snake_case 与权威设计一致）

```text
{
  implementation: 'ts' | 'rust',
  config_path: <absolute dev-home/router.yml>,
  ts_source_root: <absolute checkout/router>,   // ts only
  rust_binary_path: <absolute dev-home/bin/skiff-router>, // rust only
}
```

- `resolveRouterProcessSpec({ devHome, implementation, repoRoot, platform })`：
  唯一 canonical resolver，位于 `dev-runtime-paths.mjs`；`devHome` 必须显式传入
  （不读 ambient env 决定 implementation/paths）。
- `assertRouterProcessSpec(spec)`：严格字段集（implementation + config_path +
  对应实现唯一路径字段，禁止多余/缺失/相对路径）。
- `routerProcessInvocation(spec)`：唯一命令派生点，禁止 caller 各自判断
  pnpm/tsx/binary：
  - ts：`{ command: 'pnpm', args: ['--dir', ts_source_root, 'dev', '--config', config_path] }`
  - rust：`{ command: rust_binary_path, args: [config_path] }`

### instance config

```yaml
router:
  implementation: ts    # 或 rust；缺失默认 ts（stable hard cut 前）
```

严格校验：`router` 若出现必须是 mapping；`router.implementation` 若出现必须是
`ts` 或 `rust`；否则拒绝。解析在 `skiff-instance.mjs` 的
`loadInstance()`（`router.implementation` / `RouterProcessSpec` 解析部分），
结果挂到 `config.routerProcessSpec`。

### skiff-router binary

- `router/Cargo.toml`：package `skiff-router`，无依赖，bin `skiff-router`。
- `router/src/main.rs`：内置 SHA-256（RFC 6234 标准向量自测）；`--identity`
  输出 `skiff-router <sha256-of-self>` 后退出 0；裸调用输出
  `skiff-router: empty skeleton ... no listener bound` 到 stderr 后退出 0。
- `router/tests/identity.rs`：integration tests（`CARGO_BIN_EXE_skiff-router`）。
- workspace member `router` 恰好归入 `router-rust` subject。

### rollback manifest

- schema：`skiff-router-rollback-unit-v1`。
- 形状：`{ schemaVersion, implementation, config_path, ts_source_root | rust_binary_path, process: { command, args } }`。
- builder 消费 `RouterProcessSpec`，`process` 由 `routerProcessInvocation` 派生；
  validator 严格校验并重建 spec 复核。不实现 immutable unit 重建。

## 写集计划

可写文件（均在 worktree 内）：

| 文件 | 改动 |
| --- | --- |
| `doc/implementation/router-rust-migration/execution/router-rust-migration-pr0a-leaf.md` | 本文件（新） |
| `scripts/lib/dev-runtime-paths.mjs` | `routerBinaryName`、`routerBinary`、`resolveRouterProcessSpec`、`assertRouterProcessSpec`、`routerProcessInvocation` |
| `scripts/skiff-instance.mjs` | `loadInstance` 解析 `router.implementation`；`buildComponentBinaries` rust build/install placeholder；`managedProcessSpecs`/`commandMatchesComponent` 消费 spec；`commandLooksLikeRouterRust` |
| `router/Cargo.toml`、`router/src/main.rs`、`router/tests/identity.rs` | 新 Cargo package（新） |
| `Cargo.toml`、`Cargo.lock` | workspace member `router`（lock 由 cargo 生成） |
| `scripts/lib/verify-rust-subjects.mjs` | subject `router-rust` |
| `scripts/lib/verify-selector-graph.mjs` | manual router 展开 + `router-rust-process-smoke` public leaf |
| `scripts/lib/verify-plan.mjs` | `router-tests`→`router-ts-tests`；`router-rust-process-smoke` builder |
| `scripts/lib/router-process-smoke.mjs`、`scripts/check-router-process-smoke.mjs` | implementation-neutral smoke harness（新） |
| `scripts/lib/rollback-manifest.mjs` | schema/builder/validator（新） |
| `scripts/lib/isolated-test-runtime-instance.mjs` | fixture 显式写 `router.implementation` |
| `scripts/lib/platform-source-probe-node-dependencies.mjs` | 消费 `RouterProcessSpec.ts_source_root` |
| `scripts/tests/dev-runtime-paths.test.mjs`、`scripts/tests/rollback-manifest.test.mjs`、`scripts/tests/router-process-spec.test.mjs` | 新测试 |
| `scripts/tests/verify-taxonomy.test.mjs`、`scripts/tests/verify-rust-quality.test.mjs` | selector/CI 断言同步 |
| `.github/workflows/verify.yml` | PR job 增加 fast scope |

禁止写：`router/src/router/controlPlane.ts`、loop-risk 文件、
`scripts/lib/local-instance-config.mjs`、`scripts/check-local-instance.mjs`、
AGENTS.md（repo 与 workspace）、`scripts/README.md`、`skiff-instance.mjs`
的 `routerConfigText` / `urls.routerReload` 行、`scripts/verify-cli.mjs`、
`scripts/build-runtime-stack.mjs`。

禁止操作：重启/修改 stable instance、Mongo、PM2、4004-4007 端口进程；不跑全量
`pnpm verify`。CARGO_TARGET_DIR 只用 worktree 内 target。

## 自验收矩阵

| 项 | 命令/断言 |
| --- | --- |
| Rust 单元+集成测试 | `cargo test --package skiff-router`（worktree 内 CARGO_TARGET_DIR） |
| verify 聚焦 | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` |
| manual router 展开 | `node scripts/verify.mjs --only router --list` 展开为 `router-ts-tests` + `router-rust` + `router-rust-process-smoke` |
| subject 展开 | `node scripts/verify.mjs --only router-rust --list` 仅 `router-rust:contracts` |
| registry integrity | `node --test scripts/tests/verify-taxonomy.test.mjs`（含 workspace ownership + graph transition） |
| spec/rollback 自测 | `node --test scripts/tests/dev-runtime-paths.test.mjs scripts/tests/rollback-manifest.test.mjs scripts/tests/router-process-spec.test.mjs` |
| CI 可解析 | `python3 -c 'import yaml; yaml.safe_load(open(".github/workflows/verify.yml"))'` |

## 停止条件

- 设计空洞 / 公共契约变化 / 需要新顶层配置、manifest、schema 或集中式 owner：
  返回 `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE` 并附证据。
- 兄弟 ownership 冲突：先通知 root（`/root`）。
