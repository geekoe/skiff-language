# P5-F445H-I6K independent current-scope acceptance result

状态：

```text
FAIL
I6_ACCEPTED = NO
BLOCKING_ISSUES = 2
NON_BLOCKING_FOLLOW_UPS = 2
```

I6 production current-scope接线的静态路径与各聚焦 receipt一致，但 I6R §8.7 指定给独立
acceptance 的四个完整 crate gate没有全部通过。Eval完整 gate在直接相关的 provider stream
owner归零断言上失败；Host完整 gate还稳定暴露一个与I6语义无关、但会使指定完整门禁保持红色的旧
assembly identity fixture。当前候选不能接受，也不能解除I7。

本验收没有修改production、tests、fixtures、Cargo manifest或`Cargo.lock`，没有运行I6J的
12组selector、full stage gate、network、stable/live或MongoDB。

## 1. 候选身份与独立性

| 项 | 值 |
| --- | --- |
| baseline commit | `0f076e3f04a39633f04eccab12e3831a7a79bfe6` |
| baseline tree | `b2a47daf5738d2c76cf876b081982592571cfdb9` |
| branch | `codex/p5-f445h-i6-acceptance` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6-acceptance` |
| network | `CARGO_NET_OFFLINE=true` |
| Cargo target | worktree-local `build/cargo-target` |

baseline相对I6J的merged production baseline
`f12ee51b3c77635d8d182e09152c995ae0ac35d0`只新增I6J task/result。验收开始至结束HEAD/tree
保持不变；candidate production/tests/fixtures/Cargo/lockfile为零写入。

独立阅读链：

```text
AGENTS.md
doc/implementation/package-service-contract-deployment/AGENTS.md
doc/architecture/package-service-contract-deployment.md
phase-05-ecosystem-cutover/phase-plan.md
P5-F445H-I6R-current-scope-refresh-preflight-result.md
P5-F445H-I6E-invocation-carrier-delivery-preflight-result.md
P5-F445H-I6S-service-timeout-scope-reduction-result.md
P5-F445H-I6E1/E2R/E3/E4R2/E5/E6 consumer results
P5-F445H-I6D-host-operation-current-scope-result.md
P5-F445H-I6J-current-scope-combined-probe-{resume,result}.md
```

开发与combined结论只作为证据索引；verdict由baseline production、tests、反向搜索和本次完整
crate gate独立得出。

## 2. 完成标准矩阵

| 条款 | baseline代码/结构证据 | 本次动态证据 | 判定 |
| --- | --- | --- | --- |
| E1 carrier交付 | `native_capability.rs`在projection构造时读取一次current owned control；`capabilities.rs`及HTTP/WS/time/file/Actor内部trait继续传同一个`OwnedExecutionControl` | Eval gate中5条carrier receipt均通过，随后同crate其它test失败 | 接线成立，gate未完成 |
| HTTP unary/body/SSE open | `http.rs`进入`*_with_current_scope`；`http_client_runtime.rs::await_http_lower_with_current_scope`用current scope lease与operation primitive共同监督lower | Host unit target的I6 HTTP矩阵在328/328中通过 | PASS局部 |
| WebSocket registry Pending | native三参数→Host adapter→`ConnectionRequestRegistry::install(ExecutionScope)`；registry CAS先settle并清owner | capability-context 7条与Host纵向/route测试通过 | PASS局部 |
| time | `sleep_for_millis`取得invocation control full scope并acquire lease；0 duration保持Ready | native 7条与Eval projection Pending receipt通过 | PASS局部 |
| file | Host七个入口共用`scoped_file_future`；`FileIngest`/`StagedFile`保持单一drop owner | Host/Eval file receipts通过 | PASS局部 |
| Actor control/method/spawn | control使用scoped outbound lease；method current scope与30s primitive取早者；spawn只在有效receipt后wake | Eval/Host Actor矩阵通过 | PASS局部 |
| response sink | `HttpResponseStreamCapabilityContext`以current scope lease监督capacity wait | capability-context 4条通过 | PASS局部 |
| current/outer deadline、ancestor/internal stop | consumer均保留`ExecutionScopeTerminal`，由post-await owner物化或内部传播 | HTTP/WS/time/file/Actor/response聚焦矩阵均在完整target内通过 | PASS局部 |
| normal/late winner | lower/response committed branch、registry CAS与drop fence均存在 | ready-first、late/duplicate、wrong-session与late wake测试通过 | PASS局部 |
| owner归零 | 各consumer测试断言lease/waiter/timer或registry/staging归零 | Eval完整gate的provider task全局counter断言失败 | **FAIL** |
| 公开非目标 | WS仍为三参数；无production peer cancel/`-32800`/public `CancelError`；service第一版只继承current deadline | 冻结反向搜索通过 | PASS |
| root-only/fixed fallback边界 | canonical Eval→Host路径使用current carrier；旧Host promoted/unscoped与retired outbound表面仍存在 | repo callsite搜索未发现Host promoted context进入canonical path | 当前非blocker，见§5 |

因此静态行为路径没有发现新的production blocker；最终FAIL来自必须通过但未通过的验收门禁，
其中第一项直接使owner归零证据不成立。

## 3. Blocking issues

### B1. Eval完整gate的provider stream owner归零断言并行不稳定

命令：

```bash
cargo test -p skiff-runtime-eval --locked --no-fail-fast
```

结果：exit `101`；`402 passed / 1 failed` in lib target，其它targets
`4 + 5 + 6 + 1(doc)`通过。唯一失败：

```text
assembly_execution::async_stream_cancel::current_scope_tests::
f445h_e4r_stream_provider_task_runs_real_terminal_publication_path

left: 1
right: 0
direct task execution leaves no provider task counter behind
```

这不是可忽略的无关测试：I6S复用E4 canonical service current-scope owner，I6完成标准又明确要求
normal/terminal后owner归零。该test直接读取process-global
`PROVIDER_STREAM_TASKS_ACTIVE`并断言绝对零；同一crate其它并行provider tests可以合法持有guard，
所以完整gate不能稳定证明本case自己的owner归零。

有界诊断：

```bash
cargo test -p skiff-runtime-eval \
  f445h_e4r_stream_provider_task_runs_real_terminal_publication_path \
  --locked -- --nocapture
```

单独执行`1/1 PASS`，支持“共享全局counter测试隔离缺口”的分类，不推翻完整gate的真实FAIL。
在完整gate能稳定证明相对本case baseline或使用per-case owner之前，I6 acceptance不能通过。

### B2. Host完整gate被旧v1 assembly identity fixture稳定阻断

命令：

```bash
cargo test -p skiff-runtime-host --locked --no-fail-fast
```

结果：exit `101`；unit target `328/328`通过，随后
`active_runtime_assembly`为`1 passed / 1 failed`，其余integration/doc targets
`6 + 2 + 1`通过。失败：

```text
rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent
assemblyIdentity must use skiff-runtime-assembly-v2:sha256:<64 lowercase hex>
```

fixture在`runtime/host/tests/active_runtime_assembly.rs:314-319`构造
`skiff-runtime-assembly-v1:sha256:...`，现行strict identity在测试预期的resolver reject前先
fail closed。单selector复跑仍`0/1 FAIL`，属于确定性baseline fixture漂移，不是I6 production
current-scope缺陷。

它仍是blocking evidence issue，因为I6R §8.7把Host完整crate gate明确列为本acceptance完成条件；
不能把红色完整gate记为PASS。

## 4. 完整命令ledger

| 命令 | 结果 | 覆盖 |
| --- | --- | --- |
| `cargo test -p skiff-runtime-capability-context --locked --no-fail-fast` | PASS；66 unit + 2 doc | scope façade、WS registry、response sink |
| `cargo test -p skiff-runtime-native --locked --no-fail-fast` | PASS；120 unit + 1 doc | time/native surface |
| `cargo test -p skiff-runtime-eval --locked --no-fail-fast` | **FAIL**；418 passed / 1 failed总计 | carrier、file/Actor/service/E4 owner |
| `cargo test -p skiff-runtime-host --locked --no-fail-fast` | **FAIL**；338 passed / 1 failed总计 | HTTP/WS/file/Actor与Host全域 |
| `cargo check -p skiff-runtime-capability-context -p skiff-runtime-native -p skiff-runtime-eval -p skiff-runtime-host --locked` | PASS；仅既有warnings | 四包locked接线 |
| `cargo fmt --check` | PASS | Rust格式 |
| `git diff --check` | PASS | whitespace |

额外只运行两个失败selector做一次分类；没有重跑任一完整crate，也没有重跑I6J combined selector。
第一次Eval分类命令误加`--exact`导致0匹配，不作为证据；随后去掉`--exact`得到上述非零`1/1`。

## 5. Non-blocking follow-up 与残余风险

### N1. repo-public但canonical production不可达的root-only promoted/unscoped Host表面

`runtime/host/src/capability_context/native_projection.rs`仍公开并re-export
`RuntimeNativeFileCapabilityContext`、`RuntimeNativeTimeCapabilityContext`和
`RuntimeNativeHttpClientCapabilityContext`；其中HTTP/file/Actor相关legacy调用可走request-start
root snapshot或无scope重载。`ActorClient::{get_or_create,replace,find,remove}`也保留public
unscoped入口，HTTP concrete context保留crate-private unscoped入口。

全repo调用点搜索只在该文件看到Host promoted types的定义/impl/re-export；canonical production使用
Eval crate中同名但不同的projection type，并经`eval_capability_adapter`调用`*_in_scope` /
`*_with_current_scope`。I6E preflight已把该表面列为“若证明production可达则scope expansion”；
当前没有证明可达，因此不把它升级为本次production blocker。

残余风险是外部Rust consumer或未来repo caller可能绕开canonical carrier。Skiff未发布，后续宜退休
这些export或让它们显式fail closed/要求scope，避免同名类型与fixed fallback长期存在。

### N2. retired service timeout/relay与operation primitive残留需要持续分类

`ServiceTimeoutConfig`、`OutboundServiceContext`和root-deadline逻辑仍存在于legacy Host
`outbound_service.rs`/request-context construction及test support；canonical assembly安装
`RetiredAssemblyOutboundServiceContext`，I6 combined/acceptance没有用legacy relay。
这是I6S“第一版不新增dependency/callee timeout”的残留边界，不是当前service production owner。

Actor method的固定`30_000ms`是设计保留的operation primitive；canonical path在operation start与
current effective deadline取早者。Actor method另保留request-root cancellation branch，排在full
scope lease branch之后，并由post-await checkpoint保持精确owner。HTTP的`deadline_ms`/
`frame_deadline_ms`也只在unscoped legacy入口或transport primitive plumbing中残留；canonical
adapter调用`*_with_current_scope`并向旧transport传`None`。

这些分类依赖当前调用图。若promoted/unscoped或legacy outbound重新接入production，必须重新打开I6
而不能当作兼容fallback。

## 6. 公开非目标与动态缺口

冻结搜索确认：

- `std/websocket.skiff`与公开reference中的
  `requestJsonToConnection(connectionId, method, value)`仍严格三参数；
- `$/cancelRequest`只命中Router拒绝/负例tests；`CancelError`只命中fail-closed/test fixture，
  production没有peer cancel、`-32800`或公开cancel error；
- architecture/runtime reference明确第一版没有consumer dependency/callee operation timeout，
  `policy.timeoutMs`不复用为内部service默认；
- I6证据不依赖`runtime/host/tests`中的legacy service relay。

I7拥有的真实`.skiff` source→compiler/artifact→Router/consumer、跨进程hint、generation rollout、
Agine/codex-relay、stable/live/chat smoke仍未运行；这是规定的阶段边界，不是本次缺漏。即使修复两项
门禁，I6也只证明hermetic runtime/Host current-scope能力。

## 7. Verdict 与恢复条件

```text
FAIL
I6_ACCEPTED = NO
I7_UNBLOCKED = NO
```

恢复必须由新的repair owner修正B1测试隔离/计数证据和B2 Host strict-identity fixture；本验收不建议
借助串行test thread、忽略失败或选择器替代完整gate。任何test/fixture修改都会使I6J与本result的
证据状态失效。修复合流后应先在新精确候选上运行便宜combined integration probe，再由新的独立
acceptance owner重建四crate完整gate；不得沿用本次FAIL后的局部GREEN直接宣布接受。
