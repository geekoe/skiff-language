# P4-T03：Kernel Eval Handoff

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §6.1、§6.2、§7、§9、§12、§14。
- 风险/验收组：高风险shared API integration；完成后由R01验收并冻结lane seams。
- 当前成熟度：T01/T02 implementation checkpoints；完成后推进为shared kernel checkpoint，不是稳定候选。
- 有效证据：本任务commit叠加exact T01/T02 integration state。execution-image/context/materializer API、eval
  central dispatch、Cargo或lane module shell变化会使证据失效。
- integration边界：只提交task branch，不merge main、不push。

## DAG 与执行约束

- 依赖：T01、T02均合流integration。
- 解锁：R01；R01 PASS后T04–T06并行。
- branch：`codex/p4-t03-kernel-eval-handoff`。
- worktree：`/Users/geek/workspace/skiff-p4-t03-eval-handoff`。
- 五分钟内真实edit；若T01/T02 public handoff不一致，立即报告接口缺口，不在eval复制owner。

## 写入范围

独占`runtime/eval`的assembly seam、execution context carrier、central canonical call hook、lane module roots/shells及
必要Cargo/export；可做`runtime/request/src/assembly_seam.rs`的纯typed handoff更新。另独占
`runtime/host/src/loader/assembly_admission/tests/execution/**`共享typed full-chain fixture/harness与三个空lane test
文件，不修改host production。不得实现任一具体lane，不得修改T01/T02 owner或router。

## 完成态

1. `RuntimeAssemblyEvalTarget`消费T01 execution image与T02 ActivationContext/RequestActivationContext，不再能降级成
   legacy `EvalRuntimeProgram`；缺owner/target结构化fail closed。
2. `ProgramExecutionContext`及owned continuation carrier显式携带current activation/request generation；clone/capture/
   borrow保真，无current-service TLS。
3. canonical package direct与service instruction分别进入冻结hook；legacy service symbol不能作为canonical fallback。
4.预声明ordinary/error、async/stream/cancel、callback/native三个非重叠lane模块和trait/error交接；checkpoint允许
  lane unavailable但必须typed fail closed，不能调用旧router dispatcher。
5. `eval_context`对T02 opaque callback carrier建立到T06 callback hook的exhaustive delegate；R01阶段typed fail
   closed，不能落入legacy remote carrier/router branch。
6.共享fixture使用真实`ServiceContract`/`PackageArtifact`、deployment projection、assembly resolver、typed
  load/link/admit构造provider/consumer execution input，不手写resolved binding/target；预声明ordinary/stream/
  callback lane测试文件供T04–T06独占，并冻结`typed_execution_ordinary`、
  `typed_execution_async_stream_cancel`、`typed_execution_callback_native`三个稳定测试过滤器。
7.中央`eval_context.rs`/`program_execution.rs`与fixture root的后续写入owner冻结；T04–T06通过各自模块和lane测试
  文件实现，不再争抢match/fixture owner。

## 唯一验证 ownership

```bash
cargo check -p skiff-runtime-eval -p skiff-runtime-request
cargo test -p skiff-runtime-eval assembly_execution_handoff
cargo test -p skiff-runtime-host typed_execution_fixture
rg -n 'thread_local!|task_local!' runtime/eval/src runtime/activation/src
git diff --check
```

只格式化本任务文件；不得运行lane或完整runtime gate。

## 回报

提交一个commit，回报frozen hook/API索引、lane写入表、typed fail-closed证据、命令与自验收矩阵。
