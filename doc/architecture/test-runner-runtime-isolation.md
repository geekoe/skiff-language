# Test Runner Runtime Isolation

本文定义非 live runtime-backed test 与本地 runtime stack 的隔离边界。它描述 runner 和
CLI 的内部编排契约，不改变 `test` 语法、测试发现或 effect policy。

## Ownership Boundary

普通 `skiff test` 和 Cargo workspace 中的 test-runner runtime integration harness 都不借用
开发者常驻的 router、runtime、artifact root 或 build root。Node host orchestrator 为每次 CLI
命令或每次 Cargo harness 拥有一套临时 instance：

- router HTTP/control 使用 `46000`–`46999` 内租约保护的动态端口；
- artifact、build、runtime home、pid、log 和 config 都位于同一个临时 workspace；
- Mongo 可以读取本机 `127.0.0.1:27017`，但 mongo、telemetry 和 watch 组件不由临时
  instance 启动；
- router/runtime binary 和 router source 来自执行 CLI 的当前 Skiff checkout。

仓库测试入口按测试所有权而不是子进程使用的实现语言划分。`pnpm test` 负责 Node/TypeScript
测试和 type-check，不调度 Rust workspace tests；其中 Node-owned fixture 或 integration test
仍可能调用 Cargo 构建 Rust 产物。`cargo test --workspace --no-fail-fast` 负责完整 Rust tests；
test-runner runtime integration 由 Cargo harness 进入，但会通过 Node host 启动这里定义的隔离
runtime。`pnpm verify` 只组合 Rust、Node/TypeScript 和 checker 三类 canonical scope，不复制其
底层测试列表。

Router 不接受空 artifact root。CLI 必须在 supervisor 启动前，用当前 checkout 的
`skiff-dev-sync` 向临时 artifact/build root 写入专用 bootstrap service，并显式
`--no-reload`。readiness 先通过带 service/version selector 的 isolated HTTP bootstrap 路由
触发 lazy runtime；路由响应成功后，还必须同时观察 isolated health、临时 artifact root 和
bootstrap service 的 runtime registration。health 200 本身不代表 runtime 已能 dispatch。

## Runner Contract

Node host 向 Rust runner 显式注入 `SKIFF_DEV_RELOAD_URL`、`SKIFF_TEST_ARTIFACT_ROOT` 和
`SKIFF_DEV_HOME`。live 与非 live runtime path 都在任何 health/reload 网络请求前验证 reload
URL 和 artifact root 同时存在；CLI options 高于环境变量，缺任一都 fail closed。两种模式都
不能 fallback 到 `127.0.0.1:4001`，也不能从 health 返回值推断可写 artifact root。reload
target 只接受带显式端口的 IPv4/DNS `http://` URL，以及空 path、`/` 或精确
`/__skiff/reload-artifacts`；HTTPS、默认端口、IPv6、userinfo、其它 path、query 和 fragment
均在网络前拒绝且错误不回显原 URL。

真正的 live fixture 仍使用显式 live selector，不属于 Cargo workspace 默认测试。canonical
`runtime-live` 还同时启用 `--deny-skips` 和 `--require-tests`：任意 SKIP 或零 discovered result
都使 summary 显示 `FAILED` 并返回非零。库层 live smoke 仍保留可选 SKIP 结果，严格性由显式
CLI policy 控制。

## Live Verification Ownership

Canonical live/manual 编排只从 `scripts/lib/verify-live-registry.mjs` 生成。registry 把 source
entry、invocation 和 generated phase 分开：entry 只拥有 checker script 或 fixture discovery，
invocation 声明 selector、tier、ownership、输入、可执行文件和 strict policy，phase 才包含本次
发现得到的具体命令。普通 checker registry 与 live registry 的 checker path 按计数全局恰好
登记一次；live phase 的 fixed id/idPrefix 也和普通 checker invocation 全局去重。

模块边界保持单向：`verify-live-registry.mjs` 只拥有 canonical data/schema，
`verify-selector-graph.mjs` 只拥有普通 public/composite/internal selector graph，
`verify-live-catalog.mjs` 负责跨两类 registry 的 path/id/selector namespace 校验，
`verify-live-plan.mjs` 解释 inputs、PATH prerequisite 并生成 phase。后两者不声明 selector 或
prerequisite；普通 phase builder 还必须与 selector graph 的 leaf 集合精确对应，live selector
因此不能覆盖 `rust`、`checks` 或 `checks-default` 这类已有名字。

ownership 约束如下：

- `none` 只允许 `self-test`，不得接触外部或受管实例；loop-risk health evaluator 的 hermetic
  self-test 属于 `checks-default`，默认计划恰好运行一次；
- `external` 只允许 `live/manual`，调用者显式提供并拥有 target；`runtime-live`、
  `loop-risk-health-live` 和 `loop-risk-stress-live` 属于此类；
- `managed` 只允许 `live/manual`，checker 自己创建和清理隔离资源；当前 encrypted-storage DB
  checker 属于此类，只使用临时目录和 `45000`–`45999` 端口。

registry 的 PATH prerequisite 是精确声明：runtime fixture 只要求 `cargo`/`node`，不能因
non-live cleanup 路径虚报 `mongosh` 或 `sh`；encrypted-storage DB checker 要求
`node`/`cargo`/`pnpm`/`mongod`/`mongosh`；loop-risk health 要求 `node`，stress 要求
`node`/`ps` 和从 `router/package.json` 解析的 `ws` 模块。plan/list 阶段只读检查文件类型和
executable bit，execute 前再统一复核；任一 blocker 都在首个 command 启动前聚合。所有
`live/manual` phase 仍排除在默认 verify、Node、Cargo workspace 和 CI 之外。
Runtime fixture discovery 始终执行；任何已经提供的 config、artifact root 或 reload URL 也会
逐项校验，不能因另一个输入或 PATH 工具缺失而把空 discovery/非法配置降级成 blocked plan。

loop-risk 的两个 selector 只接受 `--loop-risk-config <path>` 或 `SKIFF_LOOP_RISK_CONFIG`。
canonical JSON 顶层严格包含 `healthUrl`、`runtimeIds` 和可选 `stress`，其中 health URL 精确为
`/__router/health?detail=loop-risk`；stress profile 还严格要求 WebSocket URL、唯一正整数 PID 和
绝对 runtime log 路径。plan/list 会解析 config 并校验 log 可读性；execution preflight 会重读
config、复核 log 与 PID 存活性，再启动任何 workload。phase 只传绝对 `--config` 路径，不能把
细粒度 target/env 或 skip 选项混进 canonical 路径；stress 的 health、CPU、log 三个 gate 都
必须实际检查并返回 `checked: true`。

`skiff-test-runner` 的 runtime integration targets 是 feature-gated inner workers。默认 Cargo
只运行一个 `harness = false` wrapper；wrapper 在 Unix 上 `exec` 为 Node host，Node host 一次
启动临时 instance，再在隔离环境中用 `--no-fail-fast` 嵌套执行全部 worker。inner marker 和
Cargo target selection 必须阻止 wrapper 递归。filter、`--list`、`--nocapture` 等外层 test
harness 参数必须透传给 inner workers。worker 不能被 `#[ignore]`、stable fallback 或外层默认
target 重复执行来规避隔离。

`--service-artifact-root` 始终是只读输入。非 live runner 把输入 root 的全部 service
pointers 和所需 artifact 复制进临时 root，使 direct dependency 的 transitive runtime
closure 可用；它仍校验 publication 声明的 direct service IDs 确实存在。live mode 不扩大
复制范围。runner 调用 `skiff-dev-sync` 时必须显式传入临时 build root。

## Lifecycle And Recovery

父 Node host（CLI 或 Cargo harness）持有 supervisor，并对正常返回、测试失败、startup
失败、`SIGINT` 和 `SIGTERM` 执行同一 cleanup：

Node host 启动的 bootstrap dev-sync 和 cargo test 命令各自拥有独立进程组。中断时必须先向整组
发送终止信号并等待组内后代全部退出，超时则强制终止；命令仍存活时不能开始清理临时
artifact、build 或 runtime stack。Node child 的 abort/error 事件不等于进程已经退出。

1. 请求 supervisor 停止；
2. 无论 supervisor 状态如何，使用本次临时 config 执行 owner-verified `instance down`；
3. 用 `instance status` 确认 router/runtime stopped，再确认所有租约端口关闭；
4. 释放端口租约；
5. 只有以上步骤全部成功才删除临时 workspace。

测试错误和 cleanup 错误必须同时保留。cleanup 失败时保留 workspace 和日志路径作为诊断
证据。startup 在 config 可读且 supervisor 已尝试启动后才执行 instance ownership cleanup；
config write 或 bootstrap 阶段失败没有 owned process，不应因为读取半成品 config 制造二次
错误。

Mongo delayed database finalizer 不属于临时 stack：它只能携带合成 database 名并连接
Mongo，不能持有 artifact、runtime home、router/runtime 或 supervisor 资源。临时 stack
cleanup 不等待每个 test case 的 6.5 秒 finalizer。
