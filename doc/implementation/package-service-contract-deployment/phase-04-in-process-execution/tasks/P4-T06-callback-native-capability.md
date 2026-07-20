# P4-T06：Callback / Native Capability Execution

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §6.2、§7、§8、§12、§14。
- 风险/验收组：高风险callback/native owner/lifetime；T04–T06合流后由R02分别验收。
- 当前成熟度：R01 shared kernel；完成后是callback/native lane checkpoint。
- 有效证据：本任务clean commit及exact R01 checkpoint。callback carrier/table/hook、native adapter、recoverable
  reject surface或测试变化会使证据失效。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：R01 PASS；与T04/T05并行。
- 解锁：R02。
- branch：`codex/p4-t06-callback-native`。
- worktree：`/Users/geek/workspace/skiff-p4-t06-callback`。
- 五分钟内真实edit。不得修改compiler/source authoring；本任务从typed artifact/assembly输入验证runtime lane。
  shared capability ABI不足时回报checkpoint缺口，不自行改变公共contract。

## 写入范围

独占T02/T03预留的callback/native lane、interface invocation consumer、native explicit callback adapter，以及
`runtime/host/src/loader/assembly_admission/tests/execution/callback_native.rs`；
可在新boundary callback模块扩实现，不继续堆`binary.rs`/`recoverable.rs`。不得修改ordinary/stream、host/router、
compiler或T03中央wiring。

## 完成态

1. local `any I`或native handle只有descriptor/adapter允许时注册activation-owned opaque capability；普通detached
   materialization不能复制method table/native address。
2. capability调用验证runtime replica、owner activation、request generation、interface/adapter contract和lifetime；
   进入capability owner执行，返回后恢复provider/receiver context。
3. top-level request与stream lifetime准确；request end、stream close、cancel、owner exit使entry失效并exact-once清理。
4. expired与unavailable按descriptor映射稳定`CapabilityExpired`/`CapabilityUnavailable`，不重建、不router fallback。
5. provider只能调用contract声明operation；wrong operation/interface/owner/generation在owner executable前失败。
6. capability进入DB/spawn/queue/persistent/recoverable lane稳定失败，rebuild hook调用次数为零。
7. native没有显式adapter时operation不可用；不得把host native capability本身跨boundary。

## 最早探针与唯一验证 ownership

```bash
cargo test -p skiff-runtime-activation callback_capability
cargo test -p skiff-runtime-boundary callback_materialization
cargo test -p skiff-runtime-eval in_process_callback
cargo test -p skiff-runtime-native callback_adapter
cargo test -p skiff-runtime-host typed_execution_callback_native
git diff --check
```

必须覆盖success context switch/restore、wrong tuple、所有expiration trigger、recoverable拒绝与native adapter缺失。
host lane测试必须复用T03 typed full-chain fixture，不手写resolved target。不得运行完整gate。

## 回报

提交一个commit，回报capability preimage/状态机、context转换、persistent-lane反证、命令与自验收矩阵。
