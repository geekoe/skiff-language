# P5-F445H-I6K-R4 independent current-scope reacceptance result

状态：

```text
PASS
BLOCKING_ISSUES = 0
NON_BLOCKING_FOLLOW_UPS = 2
I6_ACCEPTED = YES
I7_UNBLOCKED = YES
```

新的独立验收 owner 在 repair 合流后的精确候选上重建了四个完整 crate suite、四包
locked check、format/diff 与纵向静态证据。全部完整 suite PASS，合计
`947 passed / 0 failed / 0 ignored`。初次 I6K 的两个 blocker均在默认并行完整 suite中真实关闭：
Eval per-task provider owner证据稳定通过，Host strict-v2 unknown assembly fixture也通过并继续覆盖
Resolve reject、committed generation与双 replica隔离。

本验收没有修改 production、tests、fixtures、Cargo manifests、`Cargo.lock`或验证工具，没有运行
I6J selector probe、full stage gate、stable/live/network或MongoDB。

## 1. 候选身份与独立性

| 项 | 值 |
| --- | --- |
| frozen baseline commit | `0b328775bcfe2414b6abf8d28a6d28f85d0f52fe` |
| frozen baseline tree | `be151f94db44550ced73e609a4a41266b67a2f6c` |
| tested task commit | `4e349780ff68a67b4c2e0a8d085fe0afe161985a` |
| tested task tree | `68a7451a5c4d567b1307898b8ff96807c8b6f8c7` |
| branch | `codex/p5-f445h-i6k-r4-reacceptance` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6k-r4-reacceptance` |
| Cargo target | worktree-local `build/cargo-target` |
| Cargo network mode | `CARGO_NET_OFFLINE=true` |
| integration owner | `/root/phase05_integration_steward` |

tested task commit相对 frozen baseline只新增本次验收 task文档；被测production、tests、
fixtures、Cargo与lockfile bit-identical。

repair merge `55992a4d494170f3fe846ea1a22dc1154beeafbe` /
`48b2812b59da4083483493de72ab0437be2ce074` 到 frozen baseline只新增R3 task/result；
非文档diff为零。R1 implementation
`f6eb9d4b017f57536b1fdf3186f7540669049300`、R2 implementation
`067f8748eec50897c6f45588d7bbea7e4a15fd15`与R3 task
`b1af01fbd8253cc44b4e037a0a900d2af132af9b`均为baseline祖先。

本 owner未参与I6开发、初次I6K验收或R3 combined probe。先从Git对象完整读取workspace/repo
规则、权威设计、I6R/I6E/I6S parent chain、各consumer result、I6J result、I6K FAIL与R1/R2/R3
结果，再独立冻结矩阵。parent verdict只作证据索引；以下 verdict由精确候选代码、完整动态gate与
反向搜索得出。

## 2. 完整动态 gate

四个完整 suite均使用默认test harness并行度，没有`--test-threads=1`、ignore、selector或局部
替代。现有合同要求的lib、integration和rustdoc target全部执行。

| crate / gate | target ledger | crate总计 | 结果 |
| --- | --- | ---: | --- |
| `skiff-runtime-capability-context` | lib `66`；doc `2` | `68/68` | PASS |
| `skiff-runtime-native` | lib `120`；doc `1` | `121/121` | PASS |
| `skiff-runtime-eval` | lib `403`；integration `4 + 5 + 6`；doc `1` | `419/419` | PASS |
| `skiff-runtime-host` | lib `328`；integration `2 + 6 + 2`；doc `1` | `339/339` | PASS |
| **合计** | 四crate全部targets | **`947/947`** | **PASS** |

精确命令：

```bash
CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=build/cargo-target \
  cargo test -p skiff-runtime-capability-context --locked --no-fail-fast
CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=build/cargo-target \
  cargo test -p skiff-runtime-native --locked --no-fail-fast
CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --no-fail-fast
CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=build/cargo-target \
  cargo test -p skiff-runtime-host --locked --no-fail-fast
```

四包production接线：

```bash
CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=build/cargo-target \
  cargo check -p skiff-runtime-capability-context -p skiff-runtime-native \
  -p skiff-runtime-eval -p skiff-runtime-host --locked
```

结果PASS；只有baseline既有dead-code、unused-import与unreachable-pattern warnings，没有dependency
resolution、lockfile写入或network访问。

卫生：

```bash
cargo fmt --check
git diff --check
```

均PASS。合同动态gate共7条命令：4条完整suite、1条四包locked check、1条fmt、1条diff。

## 3. 初次两个 blocker 的关闭

### B1：Eval provider owner计数隔离

完整Eval默认并行suite的lib target `403/403` PASS，直接包含：

```text
f445h_e4r_stream_provider_task_runs_real_terminal_publication_path
```

静态核对确认：

- `run_provider_stream`是真实唯一guard入口，直接执行与spawn执行都经过
  `ProviderStreamTaskGuard::for_task`；
- per-task `ProviderStreamTaskActivityProbe`只在`cfg(test)`保存`entered`/`active`；
- canonical case执行真实typed terminal publication后断言本task
  `entered == 1`、`active == 0`；
- production global `PROVIDER_STREAM_TASKS_ACTIVE`仍只做fetch-add/fetch-sub diagnostic；
- `#[ignore]`、`test-threads`、`serial_test`和
  `PROVIDER_STREAM_TASKS_ACTIVE.store/swap`搜索均为零。

因此本次完整并行suite不再把其它合法并发task归到本case，也没有通过reset、等待全局归零或串行化
掩盖leak。

### B2：Host strict-v2 fixture

Host完整suite的`active_runtime_assembly` integration target `2/2` PASS，直接包含：

```text
rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent
```

fixture使用`skiff-runtime-assembly-v2:sha256:<64 b>`，词法符合当前strict identity；digest仍与
fixture assembly不同。未改变的后续断言确认：

- 拒绝原因仍为`AssemblyActivationRejectReason::Resolve`；
- committed registration保持原值；
- 第二replica registration独立保持。

repair只有test fixture的v1→v2前缀变化，没有production validation、resolver或compatibility写入。

## 4. I6纵向 current-scope证据

| 条款 | 独立静态代码事实 | 完整suite中的动态事实 | 判定 |
| --- | --- | --- | --- |
| E1 carrier | Host borrowed/owned adapter都实现`execution_scope`/`derive_scope`；native projection从`context.execution().owned()`只构造一个invocation carrier，各consumer clone同一owned control | Eval五类carrier receipt全部在403个lib tests中PASS | PASS |
| HTTP | 三个Host adapter进入`dispatch_*_with_current_scope`；shared lower helper读取full scope并acquire唯一lease，current与primitive timer共同监督lower | 11条HTTP current-scope矩阵均在Host lib target PASS | PASS |
| WebSocket request | Host operation start从carrier导出scope；registry install保存`ExecutionScopeLease`，CAS settle后清pending/timer/lease；wire业务surface不变 | capability-context 7条与Host 6条scope/route/generation cases PASS | PASS |
| time | `sleep_for_millis`从invocation control取scope并acquire lease；零duration不建owner | native 7条scope矩阵与Eval projection-to-Pending case PASS | PASS |
| file | 七个direct/provider/source入口共用`scoped_file_future`的唯一lease owner；`FileIngest`/`StagedFile`各自Drop owner保留 | Host 6条与Eval真实createFromStream Pending receipt PASS | PASS |
| Actor | control/spawn与method读取current scope；真实`OutboundRequestLease`/`ActorMethodOutboundLease`保持response commit和late fence；30s只作method primitive | Eval 4条与Host 15条Actor矩阵PASS | PASS |
| response sink | `HttpResponseStreamCapabilityContext`从current execution取scope并acquire lease监督capacity wait | capability-context 4条deadline/ancestor/normal/cleanup tests PASS | PASS |
| service | canonical in-process wait继续消费caller current scope；没有新的dependency/callee timeout owner | Eval canonical service/E4 current-scope与provider tests均PASS | PASS |
| owner归零/late fence | scope lease、registry、timer、file staging、Actor response、stream capacity各有明确completion/drop owner | 四crate相关normal/current/ancestor/late cases全部PASS且无ignored | PASS |

完整suite输出实际列出并执行了上述I6/E4 tests；证据不是只由test名称搜索或parent selector ledger推断。

## 5. 公开非目标与残余表面

冻结公开非目标保持：

- `std/websocket.skiff`的
  `requestJsonToConnection(connectionId, method, value)`严格三个业务参数；
- 四个普通WebSocket send仍是同步Ready，不为scope建立虚假suspension；
- production没有peer `$/cancelRequest`、`-32800`或公开`CancelError`；
  `CancelError`残余只在legacy spelling/model fail-closed tests；
- architecture与runtime reference都明确第一版没有consumer dependency timeout或callee operation
  timeout，`policy.timeoutMs`只属于external ingress/request；
- canonical service path没有恢复legacy `ServiceTimeoutConfig`或outbound relay。

### Non-blocking N1：repo-public promoted/unscoped Host contexts

`runtime/host/src/capability_context/native_projection.rs`仍定义并由module re-export
`RuntimeNativeFileCapabilityContext`、`RuntimeNativeTimeCapabilityContext`和
`RuntimeNativeHttpClientCapabilityContext`。全repo production搜索没有它们的constructor/use
callsite；canonical路径使用Eval crate的同名projection并携带current carrier。因此当前不可达，不是
I6 blocker。

残余风险是未来Rust caller可能接回这组同名root-only表面。后续应退休这些export或令其显式要求scope；
一旦进入canonical production必须重新打开I6。

### Non-blocking N2：retired relay与root/primitive兼容分支

legacy `OutboundServiceContext`、`ServiceTimeoutConfig`与service dispatch模块仍存在；canonical
assembly明确安装`RetiredAssemblyOutboundServiceContext`，它没有dependency、timeout或start能力并fail
closed。Actor method仍保留request-root cancellation分支和30s primitive；scope lease分支先于root
branch，wire timeout取current/outer deadline与primitive最小值。HTTP旧frame deadline只留在unscoped
plumbing；canonical adapter进入`*_with_current_scope`。

这些残余依赖当前调用图分类为非目标。若retired relay、promoted context或unscoped入口重新进入
canonical production，必须重新验收，不能当作兼容fallback。

## 6. Blocking、动态缺口与 verdict

```text
BLOCKING_ISSUES = 0
```

没有发现production、test isolation/fixture、contract/evidence、baseline或环境 blocker。

I7拥有的真实`.skiff` source→compiler/File IR/artifact→Router/consumer、跨进程internal stop hint、
generation rollout、Agine/codex-relay、stable/live/chat smoke仍未运行。这是冻结的I6/I7阶段边界，
不是本验收缺漏；本结果只接受hermetic runtime/Host current-scope能力并解除I7启动。

最终结论：

```text
PASS
I6_ACCEPTED = YES
I7_UNBLOCKED = YES
```

本result提交只改变文档tree，不失效上述精确候选证据。后续任何I6 production/test/fixture、
service-scope design、Cargo依赖或验证工具变化都会使本结果失效。
