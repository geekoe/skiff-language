# P5-F445H-I6K-R3 repair combined integration probe result

状态：

```text
PASS
MERGED_PROBE_FAIL = NO
TASK_NOT_EXECUTABLE = NO
READY_FOR_I6_REACCEPTANCE = YES
I6_ACCEPTED = NO
```

Eval per-task owner-zero修复与Host strict-v2 fixture修复在同一个merged
baseline `55992a4d494170f3fe846ea1a22dc1154beeafbe` /
`48b2812b59da4083483493de72ab0437be2ce074` 上通过便宜combined integration
probe。三组selector的listing与execution精确一致，合计`7 listed / 7 passed`；
Eval+Host共同locked接线、rustfmt、diff与静态禁止面均通过。

本节点没有实现或修改production、tests、fixtures、Cargo manifests或`Cargo.lock`，
没有重复R1/R2各自拥有的完整crate gate，没有扩张为完整I6 acceptance，也没有访问
network、stable/live或MongoDB。

## 1. 候选身份与repair ancestry

| 项 | 值 |
| --- | --- |
| baseline commit/tree | `55992a4d494170f3fe846ea1a22dc1154beeafbe` / `48b2812b59da4083483493de72ab0437be2ce074` |
| task commit/tree | `b1af01fbd8253cc44b4e037a0a900d2af132af9b` / `2f824095c7fa8c1d1271424781b59fcb26cb5c34` |
| branch | `codex/p5-f445h-i6k-r3-combined` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6k-r3-combined` |
| integration owner | `/root/phase05_integration_steward` |
| network mode | `CARGO_NET_OFFLINE=true` |
| Cargo target | worktree-local `build/cargo-target` |

零worktree只读预检确认：

- baseline commit/tree与任务信封精确一致；
- R1 implementation `f6eb9d4b017f57536b1fdf3186f7540669049300`
  是baseline祖先；
- R2 implementation `067f8748eec50897c6f45588d7bbea7e4a15fd15`
  是baseline祖先；
- baseline相对共同repair parent
  `b5f991efc6b3dd191e8d73485aac03679fe6477c` 的非文档写集只有两个Eval
  provider-task文件和一个Host integration fixture；
- 指定branch和worktree path未被占用，所有三个selector均存在真实test定义。

因此任务可执行，动态证据锚定两修复已合流的精确代码树。task/result只改变文档树，不改变
被测代码状态。

## 2. 非零selector ledger

所有Cargo命令均为offline、`--locked`，exit `0`。每组先listing，再真实执行。

| 覆盖 | package / selector | list | run |
| --- | --- | ---: | ---: |
| R1直接失败路径：精确task进入guard并owner归零 | Eval `f445h_e4r_stream_provider_task_runs_real_terminal_publication_path` | 1 | 1/1 |
| R2直接失败路径：strict-v2 unknown ref到达Resolve拒绝与状态保持 | Host integration target `active_runtime_assembly` / `rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent`，`--exact` | 1 | 1/1 |
| I6核心代表路径：current carrier交付HTTP/WS/time/file/Actor lower API | Eval `f445h_i6_carrier_delivery_receipt` | 5 | 5/5 |
| **合计** | 3 selectors | **7** | **7/7** |

R1命令：

```bash
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval \
  f445h_e4r_stream_provider_task_runs_real_terminal_publication_path \
  --locked -- --list
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval \
  f445h_e4r_stream_provider_task_runs_real_terminal_publication_path \
  --locked -- --nocapture
```

真实执行为`1 passed / 0 failed / 0 ignored`，lib target中`402 filtered out`。
该case在真实typed terminal publication后观察自己的probe：
`entered == 1`且`active == 0`；结论不依赖process-global counter的瞬时值。

R2命令：

```bash
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-host \
  --test active_runtime_assembly \
  rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent \
  --locked -- --exact --list
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-host \
  --test active_runtime_assembly \
  rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent \
  --locked -- --exact --nocapture
```

真实执行为`1 passed / 0 failed / 0 ignored`，同integration target中`1 filtered
out`。strict-v2词法输入没有被admission提前拒绝，测试继续覆盖预期Resolve reject、
committed generation保持和双replica隔离。

I6核心代表命令：

```bash
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval \
  f445h_i6_carrier_delivery_receipt \
  --locked -- --list
CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval \
  f445h_i6_carrier_delivery_receipt \
  --locked -- --nocapture
```

真实执行为`5 passed / 0 failed / 0 ignored`，lib target中`398 filtered out`。
五条receipt分别覆盖HTTP unary、WebSocket request、time、file与Actor的current carrier
下传。它是I6J既有receipt的最小代表组，不替代其余11组selector。

## 3. 共同locked接线与卫生

```bash
CARGO_NET_OFFLINE=true cargo check \
  -p skiff-runtime-eval -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

结果全部PASS。locked check同时编译两个repair package及其共享dependency/type接线；
输出只有baseline既有的dead-code、unused-import与unreachable-pattern warnings，没有
dependency resolution、lockfile写入或network访问。

`cargo fmt --check`与`git diff --check`均exit `0`。ignored的
`build/cargo-target`只包含worktree-local可再生缓存，不进入提交。

## 4. 静态集成检查

R1结构搜索确认：

- `run_provider_stream`仍通过`ProviderStreamTaskGuard::for_task(&producer)`进入唯一真实
  guard边界；
- `ProviderStreamTaskActivityProbe`仍只观察精确task；
- canonical current-scope case仍断言`entered == 1`与`active == 0`；
- `#[ignore]`、`test-threads`、`serial_test`以及
  `PROVIDER_STREAM_TASKS_ACTIVE.store/swap`搜索均为零命中。

R2结构搜索确认：

- `active_runtime_assembly.rs`的unknown fixture仍使用
  `skiff-runtime-assembly-v2:sha256:<64 hex>`；
- `runtime/host/src`与该fixture中的
  `skiff-runtime-assembly-v1`搜索为零命中；
- baseline-to-candidate没有strict validation、resolver、production或兼容路径修改。

最终baseline diff除本任务与result文档外为零。production、tests、fixtures、Cargo manifests
和`Cargo.lock`均与merged baseline bit-identical。

## 5. 命令计数、边界与未运行项

合同动态命令：

```text
6 Cargo selector commands
  3 --list
  3 execution
1 Eval+Host locked check
1 cargo fmt --check
1 git diff --check
9 total contract commands
```

额外Git identity/ancestry/status/diff读取及四类`rg`静态检查只用于候选与禁止面核对，不计入
动态probe。

明确未运行：

- `cargo test -p skiff-runtime-eval --locked --no-fail-fast`；
- `cargo test -p skiff-runtime-host --locked --no-fail-fast`；
- capability-context/native完整crate gate；
- I6J全部12组/68 tests重放；
- full I6/stage gate、stable/live、network、MongoDB。

R1/R2各自完整crate gate的既有GREEN仍属于开发owner证据；本节点没有重复消费时间或把它们
冒充本次独立acceptance。

## 6. 写集与结论

实际tracked写集严格为：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I6K-R3-repair-combined-probe.md
  P5-F445H-I6K-R3-repair-combined-probe-result.md
```

```text
READY_FOR_I6_REACCEPTANCE = YES
```

这只证明两项repair在同一merged候选上没有便宜集成回归，并解除“先通过combined
integration probe再重新验收”的恢复条件。I6仍未被本节点接受，I7仍未被本节点解除；新的
独立acceptance owner必须在最终精确代码状态上重建四crate完整gate与verdict。
