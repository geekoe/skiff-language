# Test Runner Runtime Isolation

本文定义非 live runtime-backed test 与本地 runtime stack 的隔离边界。它描述 runner 和
CLI 的内部编排契约，不改变 `test` 语法、测试发现或 effect policy。

## Ownership Boundary

普通 `skiff test`、canonical Skiff 源码套件和实现测试中的 test-runner runtime integration
harness 都不借用开发者常驻的 router、runtime、artifact root 或 build root。Node host
orchestrator 按最外层测试 invocation 拥有临时 instance：普通 CLI 命令和 runtime integration
harness 各自拥有一套；`node scripts/run-skiff-tests.mjs` 为整个 canonical registry plan 只创建
一套，并在所有 entry 之间复用同一个 router / runtime 进程：

- router HTTP/control 使用 `46000`–`46999` 内租约保护的动态端口；
- artifact、build、runtime home、pid、log 和 config 都位于同一个临时 workspace；
- Mongo 可以读取本机 `127.0.0.1:27017`，但 mongo、telemetry 和 watch 组件不由临时
  instance 启动；
- router/runtime binary 和 router source 来自执行 CLI 的当前 Skiff checkout。

仓库测试按被测对象分为两个一等 domain，而不是按实现语言或子进程工具链分类：

- `skiff-tests` 只验证 checked-in Skiff 测试源码，通过 production `skiff-test-runner` 和本文
  定义的真实隔离 runtime 执行 canonical registry；
- `implementation-tests` 验证 compiler、runtime、router、telemetry、test-runner 和 tooling
  实现本身。它可以由 Cargo、Node 或跨语言 harness 执行；test-runner runtime integration
  虽由 Cargo harness 进入，仍通过 Node host 启动本文定义的隔离 runtime。

工具链只是 domain 内部的执行细节，不能成为一等 taxonomy，也不能据此把 Skiff 源码测试
遗漏在默认 non-live gate 之外。

Router 不接受空 artifact root。CLI 必须在 supervisor 启动前，用当前 checkout 的
`skiff-dev-sync` 向临时 artifact/build root 写入专用 bootstrap service（不激活、不
reload，activation 由后续显式 bootstrap/activation URL 完成）。readiness 先通过带
service/version selector 的 isolated HTTP bootstrap 路由
触发 lazy runtime；路由响应成功后，还必须同时观察 isolated health、临时 artifact root 和
bootstrap service 的 runtime registration。health 200 本身不代表 runtime 已能 dispatch。

## Runner Contract

Canonical Skiff 源码 registry 由 `scripts/lib/skiff-source-test-registry.mjs` 唯一声明，
`scripts/run-skiff-tests.mjs` 只调用一次 `runInIsolatedTestRuntime`，并在该 owner 的 `runTest`
closure 内依次执行全部 entry。每个 entry 都调用 production `skiff-test-runner` path，固定启用
`--deny-skips` 和 `--require-tests`，且不得传 `--live` 或 `--allow-network`。entry 不能自行创建
runtime，也不能指向 stable developer instance 或固定 `4000` / `4001` 端口。

复用边界包含router/runtime进程、其隔离workspace，以及普通`kind: test` service execution内有界的
assembly activation batch。对同一个test service，runner必须只执行一次Package compile、resolved
config layer读取和dependency graph解析。non-live selected cases保持discovery顺序，按`relative_path`
文件优先贪心装箱，硬上限为16：不超过上限的单文件case不可为了填满前一batch而切分；只有一个文件自身
超过16个case时才在文件内按16切分。resolved config layers和runner-owned ingress overlay在进入batch
assembly前冻结一次；后续batch只能克隆该内存snapshot，不能重新读取authored config。live显式文件执行
不采用该non-live batching规则。

每个batch内的case各自生成fresh synthetic `ServiceDeployment`、对应`ServiceContract`和gateway
entry/ingress binding，再作为多个root链接进一个`RuntimeAssembly`。runner同时为batch内每个generated
deployment构造独立snapshot分区，汇入一个`RuntimeConfigSnapshot`，并从本次隔离activation的受信输入
写入snapshot顶层target environment。每个batch只提交一次activation transaction，并列钉住assembly
ref与snapshot ref；Runtime先验证snapshot target与activation environment精确相等，再物化逐case
ConfigView。一个case只属于一个batch，使用该batch的activation generation，不拥有单独assembly、snapshot
或generation。

runner必须先完成所有batch的plan、assembly/config projection和artifact publication，之后才能开始第一
次activation。每个batch使用同一run owner下的唯一execution scope；每次projection都从调用者传入的原始
base assembly/config pair独立开始，不能把先前test batch的assembly、snapshot或generated deployment
并入下一batch。activation、readiness和root dispatch按batch顺序串行；每次candidate generation对上一个
已commit generation执行checked increment。跨batch的case result使用同一顺序ledger，后续activation、
readiness或dispatch基础设施失败必须保留全部已完成result，并把失败坐标准确指向当前case。

batch内共享assembly/snapshot record严格不等于共享deployment、ConfigView或mutable state。每个case仍
拥有独立synthetic service identity、deployment/contract/gateway entry/ingress selector、snapshot分区、
由`(testRunId, generatedTestServiceId)`系统派生的数据库、heap、inline-effect registry和execution nonce。
case finalization精确清理这些逐case资源；共享Package artifact以及各batch assembly artifact、snapshot
record和activation由该test service execution统一清理。一个case或registry entry的可变测试状态不得泄漏
到另一个case或entry。

隔离stack同时向runner提供动态business HTTP ingress URL。runner把它作为保留的runner overlay
`skiff.test.ingressUrl`写入对应generated deployment的Package-scoped snapshot分区，并拒绝authored
同名path；它不写secret文件，也不允许
固定端口fallback。每个case的synthetic service id和contract version是self-ingress selector的唯一
来源。测试源码只传普通绝对HTTP URL；测试执行适配在URL origin精确命中该动态ingress时自动加入现有
service/version selector，用户headers不能覆盖selector、Host、body framing或hop-by-hop headers。

Router对此不增加test route、session header或business控制面旁路，只按普通business HTTP路径分发。
每次root test dispatch由Router新建一个不透明`testCaseCapability`；同一shared assembly中的不同root
必须获得不同capability。Runtime在隔离单runtime内按共享assembly identity/generation、精确case
deployment和该capability把self-ingress子请求附着到仍active的父test execution：共享inline-effect
registry，子请求不finalize，父case结束时唯一finalize。assembly/generation不能充当case identity。

`testCaseCapability`是Router/Runtime Host间的不透明私有authority，不进入Eval value、config或业务
effect surface。direct spawn、任意深度recursive spawn与Actor method spawn使用的`spawn.submit`
wire只携带`callerRequestId`；它不得重复携带`testCaseCapability`或test parent id。Router只从发送
frame的同一Runtime WebSocket session上的active parent request派生authority：父可以是root/derived
runtime-assembly request，也可以是已admit且尚未terminal的Actor invocation。同步Actor method call、
Actor get-or-create与self-ingress仍在各自的私有控制边界携带精确派生authority。Router不信任Runtime
单独提供的capability或未绑定session的parent id，不允许跨session、跨root或已结束父请求的派生。对Actor派生，首版还要求目标
service与active parent service精确相同；test capability chain不得跨service。继承链同时保持父root
固定的assembly/deployment authority，因此shared assembly不会扩大effect registry或execution nonce的
可见范围。

derived authority还不可变地钉住其activation generation。generation推进后，已active的old-generation
Actor invocation及其同步Actor call/Actor spawn仍沿原capability、parent chain、assembly/deployment和generation
执行；Router和Runtime不得把它们重绑到current generation。current-generation test Actor的self-ingress携带
该完整derived authority进入普通gateway route。old-generation Actor没有可安全使用的历史gateway route
snapshot，其self-ingress必须在dispatch前fail closed；不保留、合成、后补或构造历史snapshot，也不
借用current gateway route。

对携带test capability的Actor invocation，Router把owner约束为该派生链的精确origin Runtime id与
Runtime WebSocket connection。首次激活只能在该connection上建立owner；已有owner位于其它Runtime、origin
connection已断开/换代或dispatch后owner connection不再精确相同时必须fail closed。Runtime Host的
Router reader在同步frame handler内用capability与active parent request id取得derived case lease，并先
注册cancellation correlation，再把execution与leases移交给detached Actor owner task；两项admission中任一
失败都不得回报receipt或启动task。derived lease保留到owner task完成terminal编码/发送尝试、
失败记录并退出；因此root close可以拒绝late child，但case finalization要等待已admit Actor child的该
lease释放。

该约束依赖service-scoped、Runtime-local case registry与effect state，不定义或承诺跨service/Runtime
test-effect共享。production request、同步Actor call与Actor spawn均不携带test capability/parent id，仍按
普通Actor owner routing执行，也不能访问test case registry；其中cross-service Actor spawn仍然合法，Router不得
把test-only same-service约束应用到production。首版每个case只允许一个active self-ingress；stream在EOF、
失败或consumer drop/break后才释放active slot，并沿普通HTTP disconnect/backpressure链收束。

Node host 向 Rust runner 显式注入 `SKIFF_DEV_HOME`、`SKIFF_TEST_RUNTIME_ARTIFACT_ROOT`、
`SKIFF_TEST_ACTIVATION_URL`（精确指向 `/__skiff/activate-assembly`）、
`SKIFF_TEST_INGRESS_URL`、`SKIFF_TEST_ENVIRONMENT`、`SKIFF_TEST_EXPECTED_GENERATION` 和
`SKIFF_TEST_PLATFORM_SOURCE_ROOT`。canonical registry中
每个runner child结束后，Node host必须从同一隔离Router的health读取已settle active generation，作为下一
child的expected generation；registry index或预估batch数不能充当generation source。live 与非 live
runtime path 都在任何 health/activation 网络请求前验证 activation URL 和 artifact root 同时存在；
需要 self-ingress 的 non-live path 还必须在共享 test service assembly activation 前验证 business
ingress URL。CLI options高于
环境变量，缺任一都 fail closed。两种模式都
不能 fallback 到 `127.0.0.1:4001`，也不能从 health 返回值推断可写 artifact root。activation
target 只接受带显式端口的 IPv4/DNS `http://` URL，且 path 精确为
`/__skiff/activate-assembly`；HTTPS、默认端口、IPv6、userinfo、其它 path、query 和 fragment
均在网络前拒绝且错误不回显原 URL。stale `/__skiff/reload-artifacts` 不是当前契约。

真正的 live fixture 仍使用显式 live selector，不属于 Cargo workspace 默认测试。canonical
`runtime-live` 还同时启用 `--deny-skips` 和 `--require-tests`：任意 SKIP 或零 discovered result
都使 summary 显示 `FAILED` 并返回非零。库层 live smoke 仍保留可选 SKIP 结果，严格性由显式
CLI policy 控制。

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
Runtime fixture discovery 始终执行；任何已经提供的 config、artifact root 或 activation URL 也会
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
