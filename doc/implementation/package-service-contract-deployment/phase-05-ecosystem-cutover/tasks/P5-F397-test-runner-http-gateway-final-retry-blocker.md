# P5-F397 Test-runner HTTP gateway final retry blocker

状态：TASK_SCOPE_EXPANDED（所有授权gate通过；std activation被builtin spelling mismatch阻断）。

## 保留checkpoint

- worktree：
  `/Users/geek/workspace/skiff-p5-f386-package-test-http-gateway`
- branch：
  `codex/p5-f386-package-test-http-gateway`
- ordered prerequisites：
  `3f9b0b73` / `8329706f` / `c9e56ab2`
- provider/Clippy修复：
  `03e8192387016b82fd5af8e376b03bb90b1d7ff3`
- result/HEAD：
  `0f9bbfdb61b735f8e7342242cde9b5c550dd4cfa`
- worktree clean。

27/27 runtime execution、23+1 integration、bins、Clippy、Node receipt 4/4、fmt/diff均通过。

真实isolated在std generation 1 activation失败：

```text
std.db.ConflictError.retryable
FileIR builtin: boolean
Package schema builtin: bool
```

`runtime/linker/src/assembly_execution/service_error_index.rs::validate_type_matches_schema`按原字符串比较，
把同一语言builtin的两种拼写视为不同。该问题早于HTTP strict control执行；F397没有修改
compiler/std/linker，也已清理全部临时进程与端口。
