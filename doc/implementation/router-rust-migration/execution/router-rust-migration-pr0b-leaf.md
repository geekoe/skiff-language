# Router Rust Migration PR 0b Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界开发会话）

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-3.md`（PR 0b 节点）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5，
  重点 §5.2 C-net 与 PR 0b、§6.2(2) tooling 持续推进、§7 PR0b；冲突时以权威设计为准）。
- 直接父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-2.md`（M0 + C-net，已合入 main）。
- 冻结契约：
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-config-leaf.md`（C-config：唯一 Router
    process config schema/defaults/relative-path/redaction/unknown-key/golden corpus）。
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-net-contract.md`（C-net：Tokio multi-thread、
    hyper 1 `with_upgrades`、tokio-tungstenite 0.26、Semaphore 上限、watch + drain + deadline abort）。
  - 机制 probe：`router/tests/net_probe.rs`（C-net 真实 socket probe）。
- 仓库：`/Users/geek/workspace/skiff`；精确 baseline：`main@1d442366`
  （`git rev-parse main` = `1d442366e63e17085c4a4ab0d306627c5f494e3a`，已验证；
  预检期间 A0 已合入 `integration/router-rust-migration-batch-3`，本叶子仍以 main ref 为基线）。
- worktree：`/Users/geek/workspace/wt-pr0b`，branch `feat/router-rust-pr0b`。

## 预检结论（零 worktree 只读，锚定 main@1d442366）

### C-config / golden corpus

- Golden corpus 精确结构（`git ls-tree main -- router/tests/fixtures/router-config`）：
  `corpus.json`（`skiff-router-config-corpus-v1`，systems=`["router"]`）+ `valid/`（10 个）+ `invalid/`（48 个），
  全部 tracked。batch 文档写“invalid 47”为历史数，以 corpus.json 实际 48 为准，Rust 消费同一 JSON。
- 冻结 schema 与错误消息以 `router/src/router/config.ts`（740 行）为准；YAML 严格解析语义
  （duplicate/anchor/alias/tag 拒绝、key 必须 `[A-Za-z_][A-Za-z0-9_-]*` 且无 `.`）以
  `router/src/config/index.ts` 的 `parseStrictYamlObject` 为准。
- TS 侧 yaml 依赖使用 `schema: 'core'`；core 标量正则已从 `router/node_modules/yaml/dist/schema/core/`
  核对：int=`^[-+]?[0-9]+$|^0o[0-7]+$|^0x[0-9a-fA-F]+$`（`parseInt` 语义，f64 结果）、
  float 三种形态（普通点号、指数、`.inf/.nan`）、bool=`[Tt]rue|TRUE|[Ff]alse|FALSE`、
  null=`~|null|Null|NULL|空`。Rust 解析器必须按该 core schema 解析，而不是按 serde_yaml 的
  YAML 1.1 风格解析。
- 错误断言方式是 regex：`config-corpus.test.ts` 用 `new RegExp(error)`；Rust 测试用同 corpus 的
  `error` 字段做 regex 匹配。
- 归一化断言（canonical/minimal/renderer-canonical/aliases/manifests/telemetry/file-backend/
  direct-secrets/rewrite/numeric-strings）已在 `config-corpus.test.ts` 落盘，Rust 测试复刻同一期望表。

### YAML 解析技术选型（预检结论）

- 冻结契约要求 anchor/alias/tag 拒绝，且语义与 `yaml` npm core schema 一致。workspace lock 里可用的
  YAML 栈只有 `serde_yaml 0.9.34+deprecated` + `unsafe-libyaml 0.2.11`（均已被 workspace 既有 crate
  使用）。serde_yaml 会展开 anchor/alias、吞掉标准 tag，且标量解析偏 YAML 1.1，无法直接满足契约。
- 选择：`skiff-router` 直接依赖 `unsafe-libyaml`（已在 lock，无新版本），在其事件级 parser 上写
  最小安全 wrapper（参照 serde_yaml 已验证的同一 FFI 模式：`yaml_parser_initialize` /
  `yaml_parser_set_input_string` / `yaml_parser_parse` / `yaml_event_delete` /
  `yaml_parser_delete`；错误字段经 `yaml_parser_t: Deref<Target=yaml_parser_t_prefix>` 的 pub 前缀访问），
  然后由纯 Rust 代码实现：
  - anchor / alias / tag 事件直接拒绝（与 TS `rejectUnsupportedYamlNodeFeatures` 一致）；
  - mapping 构建时检测 duplicate key（TS `uniqueKeys: true` 归一化为 `duplicate key`）；
  - key 必须 string 且匹配 `[A-Za-z_][A-Za-z0-9_-]*`、拒绝 dotted key；
  - 按 yaml core schema 解析 plain scalar，quoted 恒为 string；
  - JSON 兼容值树（null/bool/f64 number/string/array/object）。
- 该选择不引入任何新 crate 版本，M0 closure（无 skiff-runtime-model）不受影响。

### C-net / listener 装配

- C-net 契约已冻结：hyper 1 `UpgradeableConnection::graceful_shutdown()`（必须 `.with_upgrades()`）、
  `TokioIo` 适配、`derive_accept_key` + `from_raw_socket(Role::Server)`、accept 时
  `Semaphore::try_acquire_owned()` 超限写 `503`、watch 停 accept、drain deadline 后 abort。
  机制 probe 在 `router/tests/net_probe.rs`（410 行）保持不动。
- 端口拓扑按 TS 现状（`server.ts`）：public HTTP = `httpPort`；runtime/control = `runtimePort`
  同一 socket 同时服务 control HTTP（health 占位）与 `/runtime` WS upgrade。任务书“public/runtime/
  control 三个 listener”按 C-net §2/§5 的“单一 listener 服务 HTTP 与 WS + 独立跟踪升级后的 WS 连接”
  实现为：public listener + runtime/control listener（HTTP 与 WS 共享）+ 独立跟踪的升级 WS 任务。
- 连接上限：`runtime.maxConcurrency` 驱动 runtime/control listener（pre-auth runtime 连接语义，
  见设计 §3.2/§10）；public listener 无冻结配置字段，使用常量占位上限
  `DEFAULT_PUBLIC_MAX_CONNECTIONS = 1024`，叶子内注明待 C-client-lifecycle/C-ws lane 冻结正式语义。
- shutdown：SIGINT/SIGTERM → watch → 停 accept → 连接 drain（deadline 常量 2s）→ abort 残留，
  升级后 WS 任务在 drain 后 abort；退出码 0。

### Instance 集成现状

- `scripts/lib/dev-runtime-paths.mjs`（142 行，PR 0a）：`routerBinary` /
  `resolveRouterProcessSpec` / `routerProcessInvocation` / `assertRouterProcessSpec` 已完整，Rust
  invocation = `<rust_binary_path> <config_path>`；本叶子不改。
- `scripts/skiff-instance.mjs`（1898 行，PR 0a）：
  - `buildComponentBinaries()` 在 `router.implementation: rust` 时已 build + install
    `skiff-router`（`cargo build --manifest-path router/Cargo.toml --bin skiff-router` +
    `installManagedBinary` → `dev-home/bin/skiff-router`）；但 `instance build` 不支持 `--only`，
    本叶子补 `--only router`（只构建 Router，不隐式捆绑 runtime/compiler）。
  - `routerManagedProcessSpec()`：TS 时 ports=[http, control]；Rust 时当前 `ports: []`。PR 0b
    binary 真实绑定 listener 后，Rust spec 也应声明 `ports: [http, control]`，让 `instanceStatus`
    的 running/stale-binary/port-conflict 语义与 TS 一致。
  - `commandMatchesRouterProcess()` 已支持 Rust（`commandLooksLikeRouterRust`：tokens[0] 为
    rust_binary_path 且含 config_path token）；`refreshInstanceBinaries`/`ensureManagedProcessRunning`
    按 component 独立收敛，refresh Router 不会重启 Runtime（用隔离 fixture 验证）。
- `router/src/main.rs` 当前是 PR 0a 空 skeleton（含内置 SHA-256 identity）。process smoke
  （`scripts/lib/router-process-smoke.mjs`，非本叶子写范围）断言：`--identity` 输出
  `skiff-router <sha256>` exit 0；裸调用 stderr 含 `no listener bound` 且 exit 0。PR 0b 必须保留
  这两个行为（裸调用=未提供 config path 的 marker），否则 frozen process smoke 会失败。
- 隔离 fixture 约定：`scripts/tests/managed-binary-lifecycle.test.mjs` 用临时目录 + fake
  cargo/pnpm（PATH 前置）+ 动态端口 + `router.implementation` 显式配置；本叶子按同一约定新增
  Router binary 生命周期测试。
- `check-local-instance.mjs` 只读临时 fixture + 本机路径断言，不触碰 stable instance。

## 任务边界

1. Rust config parser（`router/src/config/`）：
   - `load_router_config(path)`：读取文件 → strict YAML（duplicate/anchor/alias/tag 拒绝、
     key 模式、dotted key 拒绝）→ 冻结 schema 校验（top-level/nested unknown-key 拒绝、
     legacy 字段精确错误、必填/defaults、relative-path resolution、rewrite/fileBackend/telemetry 细节）。
   - 与 TS 相同的错误消息（`router config <dotted> ...`；YAML 错误带
     `config YAML parse error:` / `config YAML aliases|anchors|tags are not supported`）。
   - `redact_router_config()`：`serviceDb.mongoUrl`、`fileBackend.oss.accessKeyId`、
     `fileBackend.oss.accessKeySecret` → `[REDACTED]`；env 引用名不 redact。
   - 消费同一 golden corpus：`router/tests/config_corpus.rs`（valid 归一化表 + invalid regex +
     redaction）。
   - 不实现 TS overrides（CLI dev override 属 TS 侧入口，Rust binary 消费渲染后的 config 文件）。
2. Listener skeleton（`router/src/listener.rs`）：
   - public listener（`httpPort`）+ runtime/control listener（`runtimePort`：control HTTP 空响应/
     health 占位 + `runtimePath` WS upgrade 占位，无业务协议）。
   - C-net 机制完整装配：multi-thread runtime、hyper 1 `with_upgrades`、`TokioIo`、
     `derive_accept_key`/`from_raw_socket`、Semaphore 上限（超限 503）、watch + drain deadline + abort。
   - `RouterListeners`/`ListenerStartOptions`（测试可绑定 127.0.0.1:0 并读实际 addr）+
     `run_router()`（信号 → graceful shutdown → exit 0）。
   - 不实现 request dispatch、WS broker、activation transaction、runtime 注册业务。
3. Instance 集成：
   - `scripts/skiff-instance.mjs`：`instance build <config> [--only router]`；Rust spec ports 声明；
     build 输出包含 router 路径；usage 更新。
   - 隔离 fixture 验证 instance build/up 构建并安装 binary、process match、refresh Router 不重启
     Runtime（`scripts/tests/router-instance-binary-lifecycle.test.mjs`）。
4. 非目标：业务 protocol、Mongo、telemetry、控制端点业务语义、TS parser/renderer 修改、
   AGENTS.md/scripts README/verify selector graph/verify.yml 修改、
   `scripts/build-runtime-stack.mjs`/`scripts/deploy-runtime-stack.mjs`（不属于本叶子写范围，
   `--only router` 在 instance build 面闭合）。

## 写入边界

可写：
- `router/Cargo.toml`（net/async/unsafe-libyaml 从 dev-deps 提升为 production deps，按需；
  dev-deps 补 regex/serde_json，均为 lock 内既有 crate）。
- `router/src/main.rs`、`router/src/lib.rs`、`router/src/config/`、`router/src/listener.rs`。
- `router/tests/config_corpus.rs`、`router/tests/listener_skeleton.rs`、
  `router/tests/process_listener.rs`（以及 `identity.rs` 仅在行为确认不变时不动）。
- `scripts/skiff-instance.mjs`（仅 build/install/process-spec 相关）。
- `scripts/tests/router-instance-binary-lifecycle.test.mjs`（相关 fixtures/tests）。
- `doc/implementation/router-rust-migration/execution/router-rust-migration-pr0b-leaf.md`。

禁止：
- `runtime/transport/src`、`deployment`、`artifact-model`（兄弟节点）、control plane（TS）、
  config parser（TS）、AGENTS.md、scripts README、verify selector graph、verify.yml、
  `scripts/lib/verify-rust-subjects.mjs`（无新增 crate，无需注册）、
  `scripts/build-runtime-stack.mjs`、`scripts/deploy-runtime-stack.mjs`、
  stable `.skiff-instance`、Mongo、PM2、4004-4007。

## 风险与停止条件

- 冻结契约与 C-net 机制无法组合（如 corpus 需要新字段）→ 停止返回
  `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`，不自行扩 schema。
- 兄弟 ownership 冲突先通知 root。
- `unsafe-libyaml` FFI wrapper 只封装 event 读取，安全逻辑（拒绝/解析/构建）全部在 safe Rust；
  对 corpus + core schema 做单测。

## 自验收矩阵

| 项 | 命令 |
| --- | --- |
| config corpus（valid 10 / invalid 48 + 归一化 + redaction） | `cargo test --package skiff-router` |
| listener skeleton 真实 socket probe | `cargo test --package skiff-router`（listener_skeleton / process_listener） |
| verify 聚焦 | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` |
| local-instance checker | `node scripts/check-local-instance.mjs` |
| instance build/up 隔离 fixture（含 `--only router`、refresh Router 不重启 Runtime） | `node --test scripts/tests/router-instance-binary-lifecycle.test.mjs` |
| M0 closure 负例 | `cargo tree -p skiff-router -e normal` 不含 skiff-runtime-model/runtime-host/eval |
| 质量 | `cargo fmt --check`（router 范围）、`cargo clippy --package skiff-router --all-targets` |

不跑全量 `pnpm verify`，不动 stable instance / Mongo / PM2 / 4004-4007。
