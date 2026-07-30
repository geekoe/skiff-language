# P5-F445H-I6K independent current-scope acceptance

状态：`COMPLETE / FAIL`。

本节点是 I6 current-scope merged candidate 的独立只读 acceptance owner。它不采用开发或
combined-probe结论作为预设，只按权威设计、I6R frozen contract、I6S scope reduction、I6J
直接父结果和 baseline production/tests 给出最终 verdict。

## 1. 权威链与冻结候选

唯一权威设计：

```text
doc/architecture/package-service-contract-deployment.md
```

直接父节点：

```text
P5-F445H-I6J-current-scope-combined-probe-result.md
```

执行合同与范围事实继续沿以下链追溯：

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
```

冻结候选：

| 项 | 值 |
| --- | --- |
| baseline commit | `0f076e3f04a39633f04eccab12e3831a7a79bfe6` |
| baseline tree | `b2a47daf5738d2c76cf876b081982592571cfdb9` |
| branch | `codex/p5-f445h-i6-acceptance` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6-acceptance` |

baseline 相对 I6J merged production baseline `f12ee51b3c77635d8d182e09152c995ae0ac35d0`
只新增 I6J task/result；production、tests、fixtures、Cargo 和 lockfile均未改变。

## 2. 只读验收矩阵

| 条款 | 静态 owner | 必要动态证据 |
| --- | --- | --- |
| E1 同一 invocation carrier 交付 | Eval projection、capability traits、Host adapters | Eval完整 crate gate |
| HTTP unary/body/SSE open | Host scoped lower owner与primitive timeout | Host完整 crate gate |
| WebSocket request纵向 registry Pending | Eval projection→Host adapter→registry | capability-context + Host完整 crate gate |
| time | native sleep current scope lease与Eval projection Pending | native + Eval完整 crate gate |
| file | Host scoped lower、source Pending、staging drop | Host + Eval完整 crate gate |
| Actor control/method/spawn | scoped outbound lease、prepared method、spawn receipt | Eval + Host完整 crate gate |
| response sink | capability-context stream scope lease | capability-context完整 crate gate |
| current/outer deadline、ancestor/internal stop | execution scope owner与consumer tests | 四crate完整 gate |
| normal/late winner与owner归零 | CAS/completion/fence tests | 四crate完整 gate |
| current-scope公开非目标 | WS三参数；无peer cancel；service第一版scope reduction | frozen反向搜索 |
| root-only/fixed fallback残留 | legacy/public Host promoted contexts、unscoped helper、Actor/HTTP fallback分类 | production callsite与可达性搜索 |

## 3. 唯一动态命令

I6R §8.7指定本 acceptance 是以下完整 gate 的唯一 owner。全部使用
`CARGO_NET_OFFLINE=true` 与本 worktree local `build/cargo-target`，串行执行：

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

不重跑 I6J 的 12 个 selector list/run，不运行 full stage gate、stable/live/network/MongoDB。

## 4. 写入边界与停止条件

唯一允许写入本 task与配套 result。禁止修改production、tests、fixtures、Cargo manifests、
`Cargo.lock`和权威设计。任何crate失败、baseline变化、公开非目标逆转、真实production仍可达
root-only/fixed fallback，或必要条款没有非零证据时，结论为 FAIL；只记录问题，不修复。
