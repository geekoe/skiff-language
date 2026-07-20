# P4-F02：Assembly Execution Projection Repair

## 权威输入、风险与证据状态

- 执行输入：R01在`ef14a08`的blocking issue 1；T03虽解析`RuntimeAssemblyEvalTarget`，但interpreter、
  `EvalContext`与nested invocation仍只消费legacy `EvalRuntimeProgram/EvalProgramProjection`。
- 风险/验收组：高风险共享execution seam；由R01复验，不解锁具体lane。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：T03 checkpoint与R01 FAIL；可与F03/F04并行。
- 解锁：R01 retry；不得提前解锁T04。
- branch：`codex/p4-f02-assembly-execution-projection`。
- worktree：`/Users/geek/workspace/skiff-p4-f02-execution-projection`。
- 五分钟内真实edit；原T03 owner执行。若必须把assembly image转回service-specific legacy aggregate或让具体lane
  修改中央projection，立即报告`TASK_NOT_EXECUTABLE`。

## 写入范围与完成态

- 独占`runtime/eval` assembly execution projection、`Interpreter`/`EvalContext`/`ExecutableInvocation`所需的最小
  中央delegate，以及`runtime/request`纯typed handoff；不实现ordinary/stream/callback具体lane。
- assembly-backed执行必须从`AssemblyExecutionImage`解析entry、file/executable、type、const与nested call；legacy
  program只服务非assembly旧入口，canonical target不能downgrade、转换或fallback。
- T04–T06只通过已冻结lane文件/API即可执行自己的lane，无需再改central wiring。
- typed fixture必须实际构造interpreter并执行canonical executable；package/service/callback中央hook至少各有真实
  reachability证据并返回checkpoint规定的typed结果，不能只断言ready/address/provider resolution。
- 将832行fixture root按`resolver`、`scenario/fixture`、`artifacts`等聚焦支持模块拆分；lane只消费窄builder，三个
  lane文件仍为各自唯一场景写入owner。

## 唯一验证 ownership

```bash
cargo check -p skiff-runtime-eval -p skiff-runtime-request
cargo test -p skiff-runtime-eval assembly_execution_projection
cargo test -p skiff-runtime-host typed_execution_fixture
rg -n 'thread_local!|task_local!' runtime/eval/src runtime/activation/src
git diff --check
```

host过滤器必须非零执行，并区分“解析成功”与“真实executable/hook已运行”。不得运行完整runtime gate。

## 回报

提交一个clean commit，回报projection lookup矩阵、interpreter调用链、fixture拆分、动态证据与剩余lane边界。
