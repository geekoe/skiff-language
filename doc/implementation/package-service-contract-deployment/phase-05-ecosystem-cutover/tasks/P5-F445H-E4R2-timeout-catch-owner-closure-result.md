# P5-F445H-E4R2 timeout, catch and owner closure result

状态：`READY_FOR_E4R5_TIMEOUT_INPUT`。

R2 已在 implementation commit
`88209415a91d5a2bb7e6e8100ac1218b80c4947e` 闭合 timeout statement/expression、
owner materialization、ordinary catch/rethrow 与 parent current-scope 恢复。本文件为独立
result commit；为避免 commit 自引用，其精确 hash 随最终交付回报记录。

本节点只提供 R5 的 timeout/catch 输入，不代表 E4R、F445H 或 Phase 05 完成。R1 root、
E1 scope owner、公共 error、capability-context、I6 request boundary 及 R3/R4 surface 均未修改。

## 1. 写集与提交

Implementation 精确修改：

- `runtime/eval/src/eval_context/timeout.rs`
- `runtime/eval/src/program_execution/execution_scope_tests/evaluator_timeout.rs`

Result 精确新增本文。没有修改 `eval_context.rs`、其它 child、module declaration、
`exceptions.rs`、`error.rs`、E1、Cargo/manifest/lockfile或其它 task/result。

Production base 仍为 R1 implementation
`b1faea534654c2ee2109f444a6cad6b1168b8445`；implementation tree 为
`295257926506aa03fb51ac23555ddf259d6f4ebf`。

## 2. Test-first RED

在 production stub 未修改时先新增真实 selector 测试：

| 证据 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_timeout -- --list` | `11 tests, 0 benchmarks` |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_timeout -- --nocapture` | 真实 RED，`0 passed; 11 failed` |

11 个失败均穿过 `LinkedStmtIr::Timeout` 或 `LinkedExprIr::Timeout` root arm，并命中冻结的
`F445H-E4 evaluator integration is required ...` diagnostic。测试没有直接调用 timeout helper，
也没有直接构造 `ScopeTerminalCarrier` 冒充 wrapper 执行。

## 3. Production 终态

statement 与 expression helper 都从 parent `ProgramExecutionContext` clone 调用
`derive_timeout_child`，保留 child context、child scope 与 owner context 的值所有权，再分别执行：

- statement：真实 child block，原样返回 `Flow`；
- expression：真实 child expression，原样返回 `RuntimeValueCarrier`。

parent shared control 从未被跨 `await` 原地替换。normal、return、value、ordinary throw/rethrow、
0 ms、`u64::MAX`、cancel和 evaluator future drop 后，parent current scope 都保持 nesting 0；
共享 scope lifecycle 的 lease/waiter/timer 计数均归零。

只有 `RuntimeError::ScopeTerminal` 进入 owner 判断：

1. carrier 已由当前 child scope 拥有时，直接以 `ScopeTerminalCarrier::is_owned_by` 精确确认；
2. inner wrapper 传回 inherited carrier 时，保留的 owner context执行一次零单位
   `GeneratedChunk` checkpoint；
3. checkpoint 只在当前 wrapper重新观察到 local owner且再次通过 `is_owned_by` 时物化；
4. inherited request/outer terminal仍返回原 carrier；同 poll ancestor cancel由 checkpoint
   返回 `Cancelled` 并优先；
5. 非 scope error和 instruction-limit等既有 `ExecutionBudgetExceeded` 不进入物化路径。

物化结果固定为 `RuntimeError::UserException`，payload identity 为
`PlatformBuiltinErrorIdentity::Timeout`。payload details完整保留：

- `reason=deadlineExceeded`
- `deadlineSource=scope`
- `deadlineNesting`
- 完整 `deadlineSite`

exception source、local stack frame与 correlation使用当前 timeout wrapper位置和当前 request
sequence。Production 没有调用 `ordinary_catch_projection()` 猜 owner，也没有把 internal carrier
放进普通 payload、wire error或 request heap。

## 4. Statement / expression、nested owner 与 catch矩阵

最终 selector 有 11 个实际 Rust test functions：

| 测试面 | 真实入口与断言 |
| --- | --- |
| statement normal / return / max | statement root执行空 child block与 return child block；`u64::MAX`安全，body `Flow`原样返回，parent恢复 |
| expression value / max | expression root执行 child literal并返回原 value；scripted clock证明 root、derive与 child evaluator均实际执行 |
| local owner + catch | timeout child内部 `catch<TimeoutError>` 看不到 internal carrier；wrapper物化后外层 ordinary catch命中；后续 statement继续 |
| inner earlier | nesting 2 inner wrapper唯一物化；source、stack、correlation、identity及 inner `deadlineSite`精确 |
| outer earlier | inherited terminal穿过 inner；outer owner checkpoint重新观察 local terminal并唯一物化 nesting 1 |
| equal absolute deadline | tie由最外层 owner唯一物化；inner不物化 |
| request-like inherited | local duration不延长 request deadline；terminal保持 inherited/request/nesting 0，不物化且 ordinary catch miss |
| cancel same poll | scripted clock在 child checkpoint同刻触发 ancestor cancellation；返回 `Cancelled`，不生成 catchable timeout |
| 0 ms | statement child block entry与 expression child value entry分别通过真实 root arm立即终止 |
| catch / rethrow / throw path | outer ordinary catch取得 request-local exception，真实 rethrow穿过后续 timeout wrapper且保留原 owner/source/correlation |
| future drop | 真实 timeout expression进入长 native wait，首次 poll为 `Pending` 后 drop；parent scope及 lifecycle保持恢复 |

所有 local materialization 测试都断言：

- exact `TimeoutError` catch identity；
- wrapper source；
- 单一 current wrapper local stack frame；
- request trace与 `local-error:1` correlation；
- JSON round-trip后的完整 deadline details；
- parent nesting 0和默认 lifecycle snapshot。

## 5. Catch投影与 owner顺序

- wrapper内 catch：child checkpoint先产生 internal `ScopeTerminal`；ordinary catch没有
  `UserException`可匹配，因此 miss。
- 正确 owner wrapper：只在 `is_owned_by` 成立后创建 `UserException`。
- wrapper外 catch：已有 exact `TimeoutError` identity，因此 hit。
- rethrow：保留同一 `RequestException`，后续 timeout wrapper不重新包装非 scope error。
- inner earlier：inner materialize，outer原样透传 user exception。
- outer earlier / equal：inner透传 inherited carrier，outer唯一 materialize。
- request deadline：当前 wrapper owner checkpoint仍观察 inherited/request source，返回原 carrier。

## 6. 最终验证

所有命令均在独立 target
`/Users/geek/workspace/skiff-p5-f445h-e4r2-timeout/build/cargo-target` 上执行：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_timeout -- --list` | PASS，11 tests |
| `cargo test -p skiff-runtime-eval --locked f445h_e4r_timeout -- --nocapture` | PASS，11/11 |
| `cargo check -p skiff-runtime-eval --tests --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

check/test 只报告写集外基线 warning：compiler/linker dead code/unused imports、
`service_error_channel.rs` unreachable pattern，以及 ordinary tests 的 unused import。本节点写集
没有新增 warning。

按任务约束没有运行完整 eval、其它 E4R selector、prepared owner/DB selector、stable、live、
network或 MongoDB。

## 7. 反向搜索与未决问题

- `timeout.rs` 中冻结的两条 `F445H-E4 evaluator integration is required` diagnostic为零。
- Production `timeout.rs` 中 `ordinary_catch_projection()` 为零。
- Implementation commit只包含两个授权 production/test文件。
- 没有修改或新增 public error、wire、request heap、capability-context或 compatibility path。

本节点写集内无未决 blocker，没有触发 `TASK_SCOPE_EXPANDED`。以下变化会使本结果失效并要求
R5重新接收/验证：

- R1 timeout root dispatch、checkpoint或 test current-scope fixture变化；
- E1 `derive_timeout_child`、`ScopeTerminalCarrier::is_owned_by`、deadline tie或 cancel priority变化；
- `PlatformBuiltinErrorIdentity::Timeout`、ordinary catch/rethrow或 request exception
  source/stack/correlation契约变化。
