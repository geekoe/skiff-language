# Skiff Language

Skiff 是面向后端服务的语言和 runtime stack。这个仓库包含语言实现、runtime、router、telemetry、CLI 脚本、标准库源码和 canonical 文档。

本语言尚未发布，不需要兼容历史格式。修改实现时优先让语义和文档收敛到当前正确模型，不要为旧 artifact、旧配置或旧 CLI 形态新增兼容层，除非已有测试明确要求当前行为。

## 仓库入口

- 文档入口：`doc/README.md` 和 `doc/overview.md`。
- 语言规范：`doc/reference/`。
- 长期架构契约：`doc/architecture/`。
- CLI 入口：`scripts/skiff.mjs`。
- Rust workspace：仓库根 `Cargo.toml`；`router/` 是 Rust workspace crate
  `skiff-router`。
- TypeScript packages：`telemetry/`、`scripts/`、`vscode/`。
- Skiff 标准库源码：`std/` 和 `prelude/`。

## 开发约定

- 保持改动聚焦，不要顺手重排无关代码或文档。
- 文件已经很长或模式重复时，先考虑职责边界和抽象是否需要调整。
- 新增公共语义时，同时更新对应 `doc/reference/` 或 `doc/architecture/` 文档。
- 不要提交本地状态、构建产物、secret 配置、package store、runtime home、截图或浏览器 profile。
- 被忽略的本地覆盖文件包括 `.skiff-instance/`、`.skiff-package-store/`、`skiff.local.yml`、`router/router.yml`、`runtime/runtime.yml`、`target/`、`node_modules/` 和 `build/`。

## 本地语言实例

开发 compiler、runtime、router 或 telemetry 时，可以为当前 worktree 创建独立本地 instance：

```bash
node scripts/skiff.mjs instance init .skiff-instance/config.yml
node scripts/skiff.mjs instance up .skiff-instance/config.yml
node scripts/skiff.mjs instance status .skiff-instance/config.yml
```

默认生成端口：

- `ports.base + 0`（默认 `4100`）：service HTTP。
- `ports.base + 1`（默认 `4101`）：router control/runtime WebSocket。
- `ports.base + 2`（默认 `4102`）：telemetry。

MongoDB 是本机共享开发基础设施，不随 `ports.base` 偏移；默认仍是
`ports.mongo: 27017`。其他 worktree instance 保持默认的
`components.mongo: disabled` 并复用该端点。受管 MongoDB 在
`exec` 前把 open-file soft limit 提升到 65536。如果要让两个目录 instance
同时运行，创建第二个 instance 后，在它的配置文件中使用不同起始端口：

```yaml
ports:
  base: 4200
  mongo: 27017
```

结束后关闭：

```bash
node scripts/skiff.mjs instance down .skiff-instance/config.yml
```

如果改动 runtime、artifact identity、artifact schema、native signature、runtime protocol 或 artifact 加载语义，使用 `instance up` 构建并启动。它会校验受管 runtime binary 的 SHA-256 identity；磁盘 binary 已变化时会自动精确重启该 instance 的 runtime，不需要再人工串联 build 与 restart：

```bash
node scripts/skiff.mjs instance up .skiff-instance/config.yml
```

只有明确需要纯 build/install 时才使用 `instance build <config>`；它会报告仍在运行旧 binary
的 PID 和 `instance refresh-binaries <config>` 恢复命令。正常本地开发使用 `instance up <config>`，
由同一份 instance config 完成构建、安装和运行状态收敛。

纯编译和单元验证不需要启动 instance：

```bash
cargo check --manifest-path runtime/Cargo.toml
cargo test --manifest-path runtime/Cargo.toml --no-fail-fast
```

## 测试入口

仓库测试按被测对象分成两个一等域，不按实现测试设施的宿主语言分组：

1. `skiff-tests` 测试 `.skiff` 源码。唯一底层入口是 `node scripts/run-skiff-tests.mjs`；它通过
   test-runner 编译测试，并在同一套件内复用一个真实 runtime 进程执行，不为每个 fixture
   单独启动 runtime。
2. `implementation-tests` 测试 Skiff 实现，按 `foundation`、`compiler`、`runtime`、
   `test-runner`、`router`、`telemetry` 和 `tooling` 被测组件展开；`router` 即
   Rust workspace crate `skiff-router`，没有 TypeScript Router 测试入口。

仓库根的权威组合入口是：

```bash
pnpm test    # 两个测试域的完整非 live 测试，不含静态质量 gate
pnpm verify  # 完整非 live 验证：tests + rust-quality + type-check + checks
```


组件 selector 可以独立运行，但完整测试使用 `tests`，不要用 `cargo test --workspace` 或旧的
Rust/Node 分组替代。Rust workspace package 到被测组件的唯一归属声明在
`scripts/lib/verify-rust-subjects.mjs`；新增 workspace crate 时必须把它归入恰好一个 subject。
`rust-quality` 分别执行 workspace rustfmt check 和 baseline-aware workspace Clippy；Clippy 当前只对
`clippy::too_many_lines` 的 checked-in baseline 做双向门禁，其他 warning 仍为 advisory。

跨语言计划只在 `scripts/verify.mjs` 中维护。`--jobs <n>` 是唯一并发参数，默认 1（串行）；
runner 运行全部选中 task 并汇总所有失败：任一 task 的失败只计入该 task 的结果，不阻止其他
task 启动或继续，全部结束后按 plan 顺序汇总，存在 failed/blocked/interrupted 时退出码为 1。
可以先审计展开后的命令而不执行：

```bash
node scripts/verify.mjs --list
node scripts/verify.mjs --only tests --list
node scripts/verify.mjs --only skiff-tests --list
node scripts/verify.mjs --only implementation-tests --list
node scripts/verify.mjs --only rust-quality --list
```

注意一点，全量verify耗时比较久，也消耗磁盘空间，应尽量避免。最好不要放在关键路径上。 

默认入口不运行 live 检查；需要时显式选择 `runtime-live`、`db-encrypted-storage-live`、
`loop-risk-health-live` 或 `loop-risk-stress-live`。compiler boundary 和受管 compiler crates
的实际 rustdoc public API 检查都属于默认 `checks` gate；`--only compiler-boundaries` 只用于
聚焦运行 source-boundary checker。loop-risk health evaluator 的 hermetic self-test 属于
`checks-default`，在默认计划中恰好执行一次；它不访问 live target。

这四个 live selector、ownership、tier、命令形态和前置工具统一声明在
`scripts/lib/verify-live-registry.mjs`，不要再在 selector、help 或普通 checker registry 中复制。
schema/data、跨 registry catalog 校验、live plan/precondition 与普通 selector graph 分别由
`verify-live-registry.mjs`、`verify-live-catalog.mjs`、`verify-live-plan.mjs` 和
`verify-selector-graph.mjs` 负责；后面三个模块只能消费 canonical registry/graph，不能复制声明。
`runtime-live` 是 `external`，只要求 PATH 中存在 `cargo`/`node`；
`db-encrypted-storage-live` 是 `managed`，要求 `node`、`cargo`、`pnpm`、`mongod` 和 `mongosh`，
并继续只使用临时目录与 `45000`–`45999` 动态端口。两个 loop-risk selector 也是 `external`：
health 要求 `node`，stress 要求 `node`、`ps` 和从 `scripts/package.json` 解析的 `ws` 模块。四者
tier 均为 `live/manual`，默认 verify、`pnpm test`、Cargo workspace 和 CI 都不展开它们。

loop-risk canonical selector 必须通过 `--loop-risk-config <path>` 或
`SKIFF_LOOP_RISK_CONFIG` 传同一份 JSON config。顶层字段严格为 `healthUrl`、`runtimeIds` 和可选
`stress`；health URL 必须精确指向 `/__router/health?detail=loop-risk`。stress selector 还要求
`stress.wsUrl`、`stress.runtimePids` 和绝对路径 `stress.runtimeLogs`。plan/list 会校验 schema、
前置工具/模块和 log 文件；执行任何 workload 前会再次聚合校验 log 与 PID 存活性。生成的 task
只收到绝对 `--config` 路径，不展开 target、PID 或 log 参数。canonical stress 的 health、CPU、
log 三个 gate 必须全部返回 `checked: true`，不能传细粒度 target/env 或 `--skip-*` 绕过。
direct CLI 只用于诊断，仍不猜 stable 4001 或默认 pgrep pattern。

`runtime-live` 必须同时显式提供 runtime config、router reload URL 和 artifact root；专用
verify 参数是 `--runtime-live-config`、`--runtime-live-reload-url` 和
`--runtime-live-artifact-root`，对应环境变量均以 `SKIFF_RUNTIME_LIVE_` 开头。它不会读取
通用的 `SKIFF_DEV_RELOAD_URL`/`SKIFF_TEST_ARTIFACT_ROOT`，也不会猜测 stable 4001 或 health
返回的 artifact root。canonical runtime live task 固定启用 `--deny-skips --require-tests`，
因此 SKIP 和零测试都不是成功。

常用聚焦测试：

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only test-runner
node scripts/verify.mjs --only router
node scripts/verify.mjs --only telemetry
node scripts/verify.mjs --only tooling
node scripts/verify.mjs --only type-check
node scripts/verify.mjs --only checks
pnpm --filter @skiff/telemetry type-check
pnpm --filter @skiff/telemetry test
pnpm --dir scripts type-check
```

## Runtime Stack

Skiff release-mode 拓扑分成 router、runtime 和 artifacts：

- router 负责 service HTTP、control HTTP 和 runtime WebSocket。
- runtime 主动连接 router，注册当前 loaded service。
- artifacts 是文件系统里的不可变 build record 和 release pointer。
- release pointer 指向 immutable build id；router 根据 service + release/version 找到 build id，再把请求分发给注册了同一 build id 的 runtime。

release-mode HTTP 调用必须使用 selector headers：

- `X-Skiff-Service: <service-id>`
- `X-Skiff-Version: <release-id>`

没有 service/version selector、release 不存在、runtime 未注册都应该 fail closed。

部署 runtime stack 时使用显式远端：

```bash
node scripts/deploy-runtime-stack.mjs \
  --remote <user@host> \
  --only all \
  --http-max-request-bytes 67108864 \
  --http-max-response-bytes 8388608 \
  --runtime-binary build/cargo-target/x86_64-unknown-linux-gnu/release/runtime
```

示例 router/runtime 配置：

```yaml
artifactsPath: /opt/skiff/artifacts
serviceDb:
  mongoUrl: mongodb://127.0.0.1:27017/?replicaSet=rs0
releaseMode: true
http:
  port: 4000
  maxRequestBytes: 67108864
  maxResponseBytes: 8388608
runtime:
  port: 4001
  path: /runtime
  maxConcurrency: 256
```

```yaml
router: ws://127.0.0.1:4001/runtime
runtime-home: /opt/skiff/runtime-home
```

更新 artifacts 或 release pointer 后，通过 canonical control 端点
`/__skiff/activate-assembly` 发布新的 active assembly（本地实例控制端口为
`4001`）。CLI 形式：

```bash
node scripts/skiff.mjs assembly activate \
  --artifact-root /path/to/artifacts \
  --environment <environment> \
  --config-snapshot '<exact RuntimeConfigSnapshotRef JSON>' \
  --expected-generation <n> \
  --activation-url http://127.0.0.1:4001/__skiff/activate-assembly
```

stale `/__skiff/reload-artifacts` 已从当前契约移除，不作为 control reload 使用。

## 文档维护

`doc/overview.md`、`doc/reference/` 和 `doc/architecture/` 是公开文档集合。已过期的临时计划、执行记录和历史草案不要放回公开仓库；必要的稳定规则应并入 canonical 文档。
