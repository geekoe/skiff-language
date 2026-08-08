# Test Runner Runtime Isolation

本文只定义非 live runtime-backed test 的进程、store、deployment 和 capability 编排边界。
`test` 语法、发现、effect policy 与用户可见语义由
[`../reference/testing.md`](../reference/testing.md)负责；本文不重复它们。

## Ownership Boundary

普通 `skiff test`、canonical Skiff 源码套件和 test-runner runtime integration harness 都不
借用开发者常驻的 Router、Runtime、artifact root 或 build root。最外层 Node host
orchestrator 拥有临时 instance：

- Router HTTP/control 使用 `46000`–`46999` 内租约保护的动态端口；
- artifact store、build root、runtime home、pid、log 和 process config 位于同一个临时
  workspace；
- Mongo 可连接本机 `127.0.0.1:27017`，但 Mongo、telemetry 和 watch 不由临时
  instance 启动；
- Router/Runtime binary 及 Router source 来自执行 CLI 的当前 checkout。

`node scripts/run-skiff-tests.mjs` 为整个 canonical registry plan 只创建一套进程；普通 CLI
命令和 Cargo runtime integration harness 各自创建自己的 instance。entry 可复用进程和
immutable store，但不复用 case mutable state。

Router 不接受空 artifact root。supervisor 启动前，CLI 用当前 checkout 的 compiler/publisher
向临时 store 写入专用 bootstrap `ServiceDeployment`，并写入该隔离 profile 的 release
pointer。带 service/version selector 的 bootstrap HTTP 路由触发 Runtime 按 `buildId` 懒加载。
readiness 必须同时证明 Router 可路由、Runtime session 可 dispatch，以及 bootstrap build
已加载；health 200 本身不足以证明 readiness。

## Runner Contract

Canonical registry 由 `scripts/lib/skiff-source-test-registry.mjs` 唯一声明。
`scripts/run-skiff-tests.mjs` 只调用一次 `runInIsolatedTestRuntime`，并在该 owner 内顺序执行
entry。每个 entry 走 production `skiff-test-runner` path，固定启用 `--deny-skips` 和
`--require-tests`，且不得传 `--live` 或 `--allow-network`。

对同一个 `kind: test` service，runner 只执行一次 Package compile、authored config layer
读取和 dependency graph resolve。non-live selected cases 保持 discovery 顺序，按
`relative_path` 文件优先贪心装箱，每 batch 最多 16 个 case；只有单个文件自身超限
时才在文件内切分。batch 只是 runner 的调度和有界资源回收单位，不是 Runtime
artifact、不是 admission 单位。它不生成 `RuntimeAssembly`、`RuntimeConfigSnapshot` 或
activation generation，也不成为任何 execution authority。

分批前，runner 把 resolved authored config 和保留的 `skiff.test.ingressUrl` overlay
冻结在内存中；后续 batch 只能从该值投影，不重读文件。每个 case 各自产生：

- fresh synthetic service identity、`ServiceContract` 和 `ServiceDeployment`；
- 包含 case config 与 ingress overlay 的 deployment-owned `BakedConfigPayload`；
- 独立 gateway entry/ingress selector 和 release pointer；
- 精确 `buildId`、数据库 identity、request heap、effect registry 和 execution nonce。

所有 case deployment 都必须在第一次 dispatch 前完成构建、结构验证、发布和 pointer
写入。Runtime 只以每个 case 的精确 `buildId` 为 load key，按需懒加载对应 immutable
`DeploymentExecutionImage`；它不接收 batch identity，也不读取 release pointer 重新选择 build。
之后 runner 按 batch/case 顺序触发 lazy load 并 dispatch。后续 case 的 load、
readiness 或 dispatch 失败必须保留已完成 result，并把基础设施错误定位到当前 case。
一个 case 的 finalization 精确清理其 mutable resources；共享 Package artifact 和 immutable
deployment records 由最外层 execution owner 回收。

## Derived Test Authority

每次 root test dispatch 由 Router 新建一个不透明 `testCaseCapability`。case identity 至少包含
`(testRunId, generatedTestServiceId, buildId, testCaseCapability)`；batch 序号和 release pointer
都不是 authority。`testCaseCapability` 与精确 case `buildId` 共同定界 execution；任一项都不能
单独授权派生或替代另一项。capability 只存在 Router/Runtime Host 的私有传输与注册状态，
不进入 Skiff value、config 或业务 effect surface。

direct/recursive dispatch 与 Actor method dispatch 的 `task.submit.request` 只携带
`callerRequestId` 与 `TaskId`。Router 只从同一 Runtime WebSocket session 上仍 active 的父
request 派生 capability、精确 test deployment `buildId` 和 case lease；不信任 Runtime 单独
提供的 capability 或未绑定 session 的 parent id。父结束、跨 session、跨 root 或跨
service 派生均 fail closed。这一 same-service 限制只属于 test effect sharing，不改变
production 的 cross-service Actor dispatch。

派生链不可变地钉住 root case 的 deployment `buildId`。后续 release pointer 更新不
重绑已运行的 request 或 Actor。self-ingress 继承该 authority，Router 使用 immutable
deployment 中的 gateway metadata 路由到同一 build，不读取 current pointer、不保留另一份
历史 route snapshot。deployment 缺失或完整性校验失败时 fail closed。

携带 test capability 的 Actor owner 必须是派生链的精确 origin Runtime id 和 connection。
Router 先建立 cancellation correlation 和 derived case lease，再把 execution 移交给 Actor owner
task；任一 admission 失败都不得回报 receipt 或启动 task。root close 拒绝 late child，
但 finalization 必须等待已 admit child 释放 lease。首版每 case 只允许一个 active
self-ingress；EOF、失败或 consumer drop/break 后才释放 slot。

## Runner Inputs

Node host 向 Rust runner 显式注入 `SKIFF_DEV_HOME`、`SKIFF_TEST_RUNTIME_ARTIFACT_ROOT`、
`SKIFF_TEST_CONTROL_URL`、`SKIFF_TEST_INGRESS_URL`、`SKIFF_TEST_ENVIRONMENT` 和
`SKIFF_TEST_PLATFORM_SOURCE_ROOT`。不再注入 activation URL 或 expected generation。CLI options
高于环境变量；组合不完整时在任何网络请求前 fail closed。

control/ingress target 只接受带显式端口的 IPv4/DNS `http://` URL；拒绝默认端口、
userinfo、query 和 fragment，错误不回显原 URL。runner 不 fallback 到
`127.0.0.1:4001`，不从 health 推断可写 artifact root。具体 publish/pointer API 属于
control protocol，不使用已退役的 `/__skiff/activate-assembly` 或
`/__skiff/reload-artifacts`。

真正的 live fixture 仍使用显式 live selector，不属于 Cargo workspace 默认测试。canonical
`runtime-live` 同时启用 `--deny-skips` 和 `--require-tests`：任意 SKIP 或零 discovered
result 都使 summary 显示 `FAILED` 并返回非零；库层 live smoke 的可选 SKIP 由
显式 CLI policy 控制。live target 由 release profile、control URL、ingress URL 和 artifact root
唯一描述；它不接受或推导 base `RuntimeAssembly` / `RuntimeConfigSnapshot` pair。

## Live Verification Ownership

Canonical live/manual 编排只从 `scripts/lib/verify-live-registry.mjs` 生成。registry 把 source
entry、invocation 和 generated task 分开：entry 只拥有 checker script 或 fixture discovery，
invocation 声明 selector、tier、ownership、输入、可执行文件和 strict policy，task 才包含本次
发现得到的具体命令。普通 checker registry 与 live registry 的 checker path 按计数全局恰好
登记一次；live task 的 fixed id/idPrefix 也和普通 checker invocation 全局去重。

模块边界保持单向：`verify-live-registry.mjs` 只拥有 canonical data/schema，
`verify-selector-graph.mjs` 只拥有普通 public/composite/internal selector graph，
`verify-live-catalog.mjs` 负责跨两类 registry 的 path/id/selector namespace 校验，
`verify-live-plan.mjs` 解释 inputs、PATH prerequisite 并生成 task。后两者不声明 selector 或
prerequisite；普通 task builder 还必须与 selector graph 的 leaf 集合精确对应，live selector
因此不能覆盖 `compiler`、`checks` 或 `checks-default` 这类已有名字。

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
`live/manual` task 仍排除在默认 verify、所有普通 non-live selector 和 CI 之外。
Runtime fixture discovery 始终执行；任何已经提供的 config、artifact root 或 control URL 也会
逐项校验，不能因另一个输入或 PATH 工具缺失而把空 discovery/非法配置降级成 blocked plan。

loop-risk 的两个 selector 只接受 `--loop-risk-config <path>` 或 `SKIFF_LOOP_RISK_CONFIG`。
canonical JSON 顶层严格包含 `healthUrl`、`runtimeIds` 和可选 `stress`，其中 health URL 精确为
`/__router/health?detail=loop-risk`；stress profile 还严格要求 WebSocket URL、唯一正整数 PID 和
绝对 runtime log 路径。plan/list 会解析 config 并校验 log 可读性；execution preflight 会重读
config、复核 log 与 PID 存活性，再启动任何 workload。task 只传绝对 `--config` 路径，不能把
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
closure 可用；它仍校验`package.yml.services`声明的direct service IDs确实存在。live mode不扩大
复制范围。runner 调用 `skiff-dev-sync` 时必须显式传入临时 build root。

## Lifecycle And Recovery

父 Node host（普通 CLI、canonical registry runner 或 Cargo harness）持有 supervisor，并对
正常返回、测试失败、startup 失败、`SIGINT` 和 `SIGTERM` 执行同一 cleanup。canonical entry
失败时停止后续 entry，但 runtime lifecycle 仍回到同一个 owner 完成 cleanup：

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
