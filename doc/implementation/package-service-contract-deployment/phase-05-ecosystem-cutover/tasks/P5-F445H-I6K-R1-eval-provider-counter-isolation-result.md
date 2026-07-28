# P5-F445H-I6K-R1 Eval provider counter isolation result

状态：

```text
PASS
B1_CLOSED = YES
I6_ACCEPTED = NO
```

本节点关闭
`P5-F445H-I6K-independent-current-scope-acceptance-result.md` 的 B1：canonical provider
stream owner-zero测试不再把其它并行测试合法持有的 process-global
`PROVIDER_STREAM_TASKS_ACTIVE`归到本case。Host B2、repair batch集成、combined probe和新的独立
acceptance仍由后续owner负责；本结果不接受I6，也不解除I7。

## 1. 身份与实际写集

| 项 | 值 |
| --- | --- |
| baseline commit/tree | `b5f991efc6b3dd191e8d73485aac03679fe6477c` / `48d82258e3698055916ac5541f3764fe1e8a0bc1` |
| implementation commit/tree | `f6eb9d4b017f57536b1fdf3186f7540669049300` / `49170083ea60647f05b8c80f3812441e562d66f8` |
| branch | `codex/p5-f445h-i6k-r1-eval-counter` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6k-r1-eval-counter` |
| integration owner | `/root/phase05_integration_steward` |
| network mode | `CARGO_NET_OFFLINE=true` |

Implementation实际写集：

- `doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F445H-I6K-R1-eval-provider-counter-isolation.md`
- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- `runtime/eval/src/assembly_execution/async_stream_cancel/current_scope_tests.rs`

本result是第四个且唯一额外写入文件。没有修改Host、Cargo manifest、`Cargo.lock`、public API、
stable/live配置或外部状态。

`git merge-base --is-ancestor <baseline> <implementation>`成功。`git diff --quiet
0f076e3f04a39633f04eccab12e3831a7a79bfe6 <baseline> -- runtime/eval Cargo.toml Cargo.lock`
也成功，证明父验收真实RED对应的production/test/Cargo输入与本任务baseline一致；中间只有验收记录。

## 2. RED分类证据

父验收在同一Eval production/test tree上运行：

```bash
cargo test -p skiff-runtime-eval --locked --no-fail-fast
```

得到真实full-suite RED：

```text
418 passed / 1 failed
f445h_e4r_stream_provider_task_runs_real_terminal_publication_path
left: 1
right: 0
```

在本worktree第一次production修改前，同一命令运行两次均为`419/419 PASS`
（unit `403`、integration `4+5+6`、doc `1`）。这不是把baseline记为稳定GREEN，而是同一
production/test tree对相同完整命令产生RED与GREEN两种结果的直接调度不稳定证据。没有为追求本地失败继续
重复昂贵gate，也没有伪造RED。

静态原因与动态结果一致：

- 旧canonical test直接调用`run_provider_stream`，该路径没有安装guard；
- test随后读取process-global counter并断言绝对`0`；
- 其它并行provider-stream test可以合法使该counter为`1`；
- 单selector在父验收及本任务均`1/1 PASS`。

没有发现production leak、共享runtime owner变化或设计缺口。

## 3. 修复

`ProviderStreamTaskGuard`现在由`run_provider_stream`安装，因此spawned和测试直接执行路径经过同一个真实
provider-task生命周期边界。全局counter仍按原语义加一/减一，production执行与terminal publication没有
变化。

仅在`cfg(test)`下，`ProviderStreamTask`可以携带一个per-task
`ProviderStreamTaskActivityProbe`。guard对这个精确probe记录：

- `entered`：该task进入guard的次数；
- `active`：该task当前持有的guard owner数。

canonical current-scope test给自己的task绑定fresh probe，真实运行并消费typed terminal后断言
`entered == 1`和`active == 0`。因此owner-zero断言更精确，而不是被删除或降级。原guard unit test也改为
验证fresh probe从`active == 1`回到`0`，不再对并发共享全局值做脆弱的相对快照。

全局diagnostic counter及其test-support export保留；没有reset、等待全局归零、ignore、whole-crate
serialization或隐藏leak的逻辑。

## 4. GREEN ledger

| 层级 | 命令 | 精确结果 | 覆盖 |
| --- | --- | --- | --- |
| selector | `CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval f445h_e4r_stream_provider_task_runs_real_terminal_publication_path --locked -- --nocapture` | PASS；`1 passed`，`402 filtered out` | real task guard entry、typed terminal、per-task owner-zero |
| full crate | `CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-eval --locked --no-fail-fast` | PASS；unit `403/403`、integration `4+5+6`、doc `1`，总计`419/419` | 完整Eval并行gate |
| locked check | `CARGO_NET_OFFLINE=true cargo check -p skiff-runtime-eval --locked` | PASS；仅既有warnings | 非test production build与locked依赖 |
| format | `cargo fmt --check` | PASS | Rust格式 |
| whitespace | `git diff --check` | PASS | task/result前implementation diff |

完整Eval gate在实质修复后只运行一次。它之后的唯一Rust变化是rustfmt要求的同一import折行；没有语义或
编译输入变化，随后fmt/diff均通过。

## 5. 反向搜索与自验收

以下搜索均为零命中：

```bash
git grep -n 'PROVIDER_STREAM_TASKS_ACTIVE.*\(store\|swap\)' <implementation> -- runtime/eval
git grep -n '#\[ignore\]\|test-threads\|serial_test' <implementation> -- \
  runtime/eval/src/assembly_execution/async_stream_cancel.rs \
  runtime/eval/src/assembly_execution/async_stream_cancel/current_scope_tests.rs
```

| 任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| 本case精确owner归零 | `ProviderStreamTaskActivityProbe`与canonical current-scope test的`entered/active`断言 | canonical test不再读取global counter | selector `1/1`、full `419/419` |
| 真实生命周期路径 | `run_provider_stream`安装`ProviderStreamTaskGuard::for_task` | spawn wrapper不再拥有第二个guard | full Eval gate |
| production/public语义不变 | probe字段、类型和分支均为`cfg(test)`；global increment/drop保留 | 无Host/Cargo/public写入 | locked check |
| 不掩盖leak/不串行化 | fresh per-task probe只观察，不reset或等待 | ignore/serialization/reset搜索零命中 | full并行gate |

## 6. 交接

```text
B1_CLOSED = YES
READY_FOR_INTEGRATION = YES
```

集成owner应核对implementation/result提交身份和四文件写集，串行合入repair batch。B2合流后按父验收恢复
条件运行combined integration probe，再由新的独立acceptance owner重建四crate完整gate。本节点不要求也
未运行Host/full-stage/stable/live/network/MongoDB。
