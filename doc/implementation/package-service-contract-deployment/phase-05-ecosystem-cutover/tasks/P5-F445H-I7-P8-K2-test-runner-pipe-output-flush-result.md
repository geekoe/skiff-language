# P5-F445H I7 P8 K2 test-runner pipe output flush result

状态：

```text
PASS
TASK_SCOPE_EXPANDED = NO
READY_FOR_INTEGRATION = YES
```

## Baseline and preflight

- baseline：
  `6bfddf0bcb45398239f26ed0fdff74c047774d34`
- baseline tree：
  `3f47d5be3009649441e8eb8433a4747b01a3e608`
- 分类：
  `TEST_RUNNER_LOCAL_OUTPUT_LIFECYCLE_DEFECT`

零worktree预检确认：

- `test-runner/src/main.rs::execute`已在失败判定前生成全部逐case stdout；
- `main`随后输出原有`error: N test(s) failed`与usage，却调用`process::exit(1)`立即终止；
- 当前没有pipe捕获多case失败输出的回归；
- 修复不需要跨crate、公共协议或CLI格式变化。

## Implementation

- production `main`改为正常返回`ExitCode`，不再调用`process::exit`。
- stdout/stderr均由`main`锁定并传入现有输出路径；返回退出码前显式flush两个writer。
- 抽出`report_summary`只为了让生产输出与pipe回归消费同一实现；逐case、message、失败摘要、
  usage及成功摘要文本和stdout/stderr归属均未改变。
- pipe子进程使用`BufWriter`主动制造非行缓冲环境，并在flush后立即退出：
  - 一条PASS、两条FAIL及两条message全部存在；
  - stderr精确保留`error: 2 test(s) failed`与usage；
  - failure退出码为1；
  - success仍输出原成功摘要、stderr为空、退出码为0。
- 测试代码单独放在`test-runner/src/main/tests.rs`，避免继续扩大CLI production文件。
- 未引入sleep、隐藏CLI参数、测试专用production协议或外部状态。

## Evidence

```text
cargo test --locked -p skiff-test-runner --bin skiff-test-runner
PASS: 3 passed, 0 failed

cargo test --locked -p skiff-test-runner --no-fail-fast
ATTEMPTED: 2 unrelated failures

cargo test --locked -p skiff-test-runner --no-fail-fast -- \
  --skip explicit_test_http_entries_cross_the_real_isolated_router \
  --skip canonical_live_source_roots_compile_to_current_receipts
PASS: 78 passed, 0 failed, 3 ignored, 2 filtered out

cargo fmt --all -- --check
PASS

git diff --check
PASS
```

完整test-runner命令的两个失败均不经过本次输出改动：

1. `explicit_test_http_entries_cross_the_real_isolated_router`在启动fixture时失败，因为worktree没有
   `node_modules`，Router日志为`tsx: command not found`。按任务边界未安装依赖、未运行网络fixture。
2. `canonical_live_source_roots_compile_to_current_receipts`的checked-in package/deployment/assembly
   identity已与指定baseline的实际编译值不一致。本任务只修改binary `main.rs`及其测试，不参与
   package artifact identity计算；未越界更新golden。

## Actual write set

```text
test-runner/src/main.rs
test-runner/src/main/tests.rs
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-P8-K2-test-runner-pipe-output-flush.md
  P5-F445H-I7-P8-K2-test-runner-pipe-output-flush-result.md
```

未运行Agine、stable、Mongo、OAuth、browser或外部网络；未push。
