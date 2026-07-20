# P4-F04：Assembly Linker Call Semantic Validation Repair

## 权威输入、风险与证据状态

- 执行输入：R01在`ef14a08`的blocking issue 3；新assembly linker traversal未复用legacy linker的native signature、
  interface method slot/ABI与const receiver ABI验证。
- 风险/验收组：高风险link/admission fail-closed；由R01复验，不解锁T04。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：T01 checkpoint与R01 FAIL；可与F02/F03并行。
- 解锁：R01 retry。
- branch：`codex/p4-f04-assembly-linker-call-validation`。
- worktree：`/Users/geek/workspace/skiff-p4-f04-linker-validation`。
- 五分钟内真实edit；原T01 owner执行。不得以复制legacy match或request-time validation修复漂移。

## 写入范围与完成态

- 独占`runtime/linker` call semantic validation与两条link traversal的delegate/tests；必要时在该crate内新增聚焦模块。
  不修改linked-program public ABI、eval/activation/boundary/host/router。
- 提取/复用单一validator，至少覆盖native target signature、interface method operation/slot/ABI、
  `LocalConstReceiverExecutable` receiver ABI与target；legacy与assembly linker必须共同调用。
- recompute identity后的malformed native/interface/receiver fixture必须在assembly execution image/admission前fail closed；
  正常builtin/native/interface call不回归。
- 删除重复验证分支或证明只剩薄delegate；不得让两套traversal再次拥有同一语义规则。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-linker assembly_execution_call_validation
cargo test -p skiff-runtime-linker call_semantic_validation
node scripts/check-runtime-crate-dag.mjs
git diff --check
```

不得运行完整runtime gate。

## 回报

提交一个clean commit，回报共享validator入口、legacy/assembly delegate矩阵、malformed负例与命令结果。
