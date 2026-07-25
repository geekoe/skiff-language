# P5-F299 Runtime local carrier第一次执行结果

状态：`TASK_NOT_EXECUTABLE`。

代码状态：`6daea926`。

## 直接任务

- `P5-F299-runtime-local-exception-carrier.md`
- 该任务继续引用F297、F284与F280父链。

## 阻塞事实

F299授权的`runtime/model/**`与`runtime/eval/**`不是required instruction facts的owner。
在artifact进入eval之前，当前linked转换存在以下信息损失：

- `runtime/linked-program/src/linked.rs`中的`LinkedStmtIr::Throw`与
  `LinkedExprIr::Throw`没有`InstructionSourceSite`；
- linked `CallIr`没有required `site`；
- `LinkedExprIr::Catch.catch_type`仍是optional并带serde default；
- `runtime/linker/src/linker/file_conversion.rs`没有复制artifact throw/call的required site，
  并把required artifact catch降成optional；
- `runtime/linker/src/assembly_execution/code_linker.rs`仍按optional catch处理。

因此F299不能在范围内创建真实throw source、local call stack或required exact catch，也不能从
display、shape或静态上下文推断/伪造这些事实。

最小前置是`P5-F300-linked-exception-sites.md`。该节点完成后，F299从新检查点重新派发。

## 执行证据

- `cargo test -p skiff-runtime-model --lib -- --list`：PASS，列出76个测试；
- `cargo test -p skiff-runtime-eval --lib -- --list`：被并行F296尚未迁移的
  compiler/core AppliedNominal编译错误遮挡；
- `git diff --check`：PASS；
- worktree恢复clean，无production提交；
- 未push，未操作stable/live。

