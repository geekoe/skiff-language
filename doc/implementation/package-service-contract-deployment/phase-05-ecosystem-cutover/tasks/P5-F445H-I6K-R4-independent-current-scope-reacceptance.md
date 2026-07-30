# P5-F445H-I6K-R4 independent current-scope reacceptance

状态：`READY`。

本节点由未参与 I6 开发、初次 I6K FAIL 或 repair combined probe 的新验收 owner
执行。它在两项 repair 已合流且 R3 combined probe PASS 后，独立重建 I6 最终证据并给出
PASS/FAIL；开发结果与 combined 结果只作证据索引，不预设 verdict。

## 1. 权威链与精确候选

唯一权威设计：

```text
doc/architecture/package-service-contract-deployment.md
```

直接恢复条件：

```text
P5-F445H-I6K-independent-current-scope-acceptance-result.md
P5-F445H-I6K-R1-eval-provider-counter-isolation-result.md
P5-F445H-I6K-R2-host-runtime-assembly-v2-fixture-result.md
P5-F445H-I6K-R3-repair-combined-probe-result.md
```

I6 执行合同与 parent evidence：

```text
P5-F445H-I6R-current-scope-refresh-preflight-result.md
P5-F445H-I6E-invocation-carrier-delivery-preflight-result.md
P5-F445H-I6S-service-timeout-scope-reduction-result.md
P5-F445H-I6E1-shared-carrier-delivery-checkpoint-result.md
P5-F445H-I6E2R-http-current-scope-resume-result.md
P5-F445H-I6E3-websocket-current-scope-resume-result.md
P5-F445H-I6E4R2-time-eval-fixture-closure-result.md
P5-F445H-I6E5-file-current-scope-resume-result.md
P5-F445H-I6E6-actor-current-scope-resume-result.md
P5-F445H-I6D-host-operation-current-scope-result.md
P5-F445H-I6J-current-scope-combined-probe-result.md
```

冻结候选：

| 项 | 值 |
| --- | --- |
| baseline commit | `0b328775bcfe2414b6abf8d28a6d28f85d0f52fe` |
| baseline tree | `be151f94db44550ced73e609a4a41266b67a2f6c` |
| repair merge commit/tree | `55992a4d494170f3fe846ea1a22dc1154beeafbe` / `48b2812b59da4083483493de72ab0437be2ce074` |
| branch | `codex/p5-f445h-i6k-r4-reacceptance` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6k-r4-reacceptance` |
| integration owner | `/root/phase05_integration_steward` |

`55992a4d..0b328775` 只新增 R3 task/result；production、tests、fixtures、
Cargo manifests 与 `Cargo.lock` bit-identical。R1 implementation
`f6eb9d4b017f57536b1fdf3186f7540669049300`、R2 implementation
`067f8748eec50897c6f45588d7bbea7e4a15fd15` 和 R3 task
`b1af01fbd8253cc44b4e037a0a900d2af132af9b` 都是 baseline 祖先。

## 2. 冻结验收矩阵

| 条款 | 静态 owner / 边界 | 必要动态证据 |
| --- | --- | --- |
| E1 同一 invocation carrier 交付 | Eval projection、capability traits、Host adapters | Eval 完整 crate suite |
| HTTP unary/body/SSE open | Host scoped lower owner、current/primitive winner、late fence | Host 完整 crate suite |
| WebSocket request | Eval projection→Host adapter→registry Pending；三业务参数 | capability-context + Host 完整 suite |
| time | native sleep current scope lease；Eval projection真实 Pending | native + Eval 完整 suite |
| file | Host scoped lower、source Pending、ingest/staging drop | Host + Eval 完整 suite |
| Actor control/method/spawn | scoped outbound lease、30s primitive、spawn receipt fence | Eval + Host 完整 suite |
| response sink | capability-context current scope lease与capacity wait | capability-context 完整 suite |
| service first-version scope | caller current deadline；无dependency/callee timeout或policy复用 | Eval完整 suite + 静态反向搜索 |
| current/outer deadline、ancestor/internal stop | exact execution scope owner与post-await materialization | 四 crate 完整 suite |
| normal/late winner与owner归零 | completion/CAS/drop fence；per-task/per-request counters | 四 crate 完整 suite |
| B1 repair | provider task guard + per-task activity probe；不reset/串行化 | Eval完整并行 suite必须稳定 GREEN |
| B2 repair | strict-v2 unknown assembly ref仍到达 Resolve reject | Host完整 suite必须 GREEN |
| 公开非目标 | WS三参数；无peer cancel/`-32800`/public cancel error；普通send不虚假挂起 | 静态搜索 |
| legacy/root-only residual | promoted/unscoped Host、retired service relay、fixed primitive逐项分类 | production callsite与可达性搜索 |

## 3. 唯一动态 gate

所有 Cargo 命令使用 `CARGO_NET_OFFLINE=true`、`--locked` 与当前 worktree-local
`build/cargo-target`。四个完整 suite 必须按现有合同包含 lib、integration 和 rustdoc targets；
不得用 selector、串行 test thread、ignore 或局部 GREEN 替代。

```bash
cargo test -p skiff-runtime-capability-context --locked --no-fail-fast
cargo test -p skiff-runtime-native --locked --no-fail-fast
cargo test -p skiff-runtime-eval --locked --no-fail-fast
cargo test -p skiff-runtime-host --locked --no-fail-fast
cargo check -p skiff-runtime-capability-context -p skiff-runtime-native \
  -p skiff-runtime-eval -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

完整 suite 可以在同一精确候选上并行，但不得共享其它 worktree 的 Cargo target。结果必须记录
每个 target 的 passed/failed/ignored 及 crate 总计。任一失败先精确分类，不修改候选。

## 4. 静态边界与纵向抽查

至少独立确认：

1. `RuntimeExecutionControl` / `RuntimeOwnedExecutionControl` full scope转发，以及
   `RuntimeNativeInvocationExecutionControl` 每次 invocation读取一次；
2. HTTP、WebSocket、time、file、Actor的 carrier 真实到达 lower pending owner；
3. response sink、registry、file staging、Actor registries的 normal/current/ancestor/late
   settlement和owner归零测试仍被完整 suite选中；
4. R1 的 per-task probe只在test configuration观测精确task，global diagnostic counter没有
   reset/store/swap，且没有 `#[ignore]`、`test-threads`、`serial_test`；
5. R2 fixture仍使用 strict-v2 identity，digest仍unknown，Resolve/state/replica断言未削弱；
6. `requestJsonToConnection`仍严格三业务参数，production没有peer cancel、
   `$/cancelRequest`、`-32800`或public `CancelError`；
7. service第一版没有consumer dependency/callee operation timeout，不把
   `DeploymentPolicy.timeoutMs`接入canonical internal service call；
8. promoted/unscoped Host contexts、legacy outbound/service timeout与fixed primitive残余只在
   已冻结非目标或不可达表面；若发现canonical production可达则升级 blocker。

必要搜索可使用 `git grep`、`rg`、`git diff` 与精确源码阅读。不得仅复述 parent result。

## 5. 写入与权限边界

唯一允许 tracked 写入：

```text
P5-F445H-I6K-R4-independent-current-scope-reacceptance.md
P5-F445H-I6K-R4-independent-current-scope-reacceptance-result.md
```

禁止修改 production、tests、fixtures、权威设计、Cargo manifests、`Cargo.lock`或验证工具；
禁止修问题。不得 merge/rebase/push，不操作 stable/live/network/MongoDB，不清理一级
worktree/branch。

## 6. 判定与交付

任一完整 crate suite失败、B1/B2未在完整并行 suite真实关闭、必要纵向路径缺少非零证据、
公开非目标逆转、canonical production仍可达root-only/fixed fallback，或候选身份变化，均为
FAIL。失败必须分类为 production、test isolation/fixture、contract/evidence、baseline或环境，
并给出最小恢复条件。

PASS 时必须同时报告：

```text
I6_ACCEPTED = YES
I7_UNBLOCKED = YES
```

FAIL 时二者都为 `NO`。结果需包含 gate计数、blocking/non-blocking、动态缺口、公开非目标和残余
风险，提交后保持 worktree clean，并把 branch/worktree、result commit/tree、verdict与计数直接
交付 `/root/phase05_integration_steward`。
