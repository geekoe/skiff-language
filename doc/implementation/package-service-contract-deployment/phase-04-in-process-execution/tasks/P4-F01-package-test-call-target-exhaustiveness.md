# P4-F01：Package-Test Call-Target Exhaustiveness Repair

## 权威输入、风险与证据状态

- 执行输入：T01提交`a6e154c`新增`LinkedCallTarget::PackageDirect`与
  `LinkedCallTarget::ActivationRelativeService`后，T03 host fixture编译在
  `runtime/package-test/src/executable_graph.rs::scan_call`出现non-exhaustive match。
- 风险/验收组：低风险机械API fallout；由本任务package-test聚焦测试与T03 host fixture验证覆盖，不新增独立验收。
- 当前成熟度：T01/T02已合流；T03 eval/request check通过，host typed fixture等待本修复。
- integration边界：只提交task branch，不merge integration/main、不push；主Agent合流后通知T03同步。

## DAG 与执行约束

- 依赖：blocker已由T03在integration基线复现。
- 解锁：T03 `cargo test -p skiff-runtime-host typed_execution_fixture`及R01。
- branch：`codex/p4-f01-package-test-call-target`。
- worktree：`/Users/geek/workspace/skiff-p4-f01-package-test-call-target`。
- 五分钟内产生真实代码edit；此前不跑测试。原T01 owner执行，不把package-test迁移扩成Phase 04新lane。

## 写入范围与完成态

- 只修改`runtime/package-test/src/executable_graph.rs`及该模块的直接聚焦测试。
- 对两个runtime-only call target闭合exhaustive match：package direct不得漏掉exact executable edge；
  activation-relative service若不属于package-test graph能力，必须结构化fail closed，不得当作无边、legacy symbol或
  remote/router call静默接受。
- 不修改linked-program/linker/eval/request/host、Cargo/lock或公共ABI；不新增adapter、dual path或package-test
  service execution语义。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-package-test executable_graph
cargo check -p skiff-runtime-host --tests
git diff --check
```

若host check继续被不同owner阻断，给出exact file/symbol，不越界修复。

## 回报

提交一个clean commit，回报两个variant的处理矩阵、命令结果和remaining owner blocker。
