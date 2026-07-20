# P4-F09：Stream Terminal / Drop Cleanup

## Blocker、输入与边界

R02在exact clean candidate `9809dee9f824c72da6508743caffa40099811031`复验FAIL。唯一blocker是callback-item
stream在未消费或外部边界拒绝时可能形成producer/runtime ownership环：capacity=1的channel装满item后，producer
等待terminal publication；producer持有的`Interpreter`与owned context继续持有同一`StreamRuntime`；外层runtime
drop又因clone仍在而不终止stream，导致task、registry、`RequestStreamLease`与callback capability永久active。

权威输入为架构§6–§8、§12、§14，`phase-plan.md`，P4-T05、F08、R02合同与R02@`9809dee` verdict。只修
async/stream terminal ownership、异常consumer退出与对应动态证据；不得修改callback projection identity、共享
materializer、ordinary lane、router、compiler或Wave 3 owner。

- 依赖：R02@`9809dee` FAIL。
- 解锁：R02 retry。
- branch：`codex/p4-f09-stream-terminal-drop-cleanup`。
- worktree：`/Users/geek/workspace/skiff-p4-f09-stream-cleanup`。
- integration边界：只提交task branch，不merge integration/main、不push。

## 写入范围

独占`runtime/host/src/capability_context/stream_runtime.rs`及其测试、
`runtime/eval/src/assembly_execution/async_stream_cancel.rs`、所有消费runtime response stream且可能提前返回的最小
consumer路径，以及host typed async/stream动态测试。避免继续扩张共享`execution/artifacts.rs`；新lifecycle fixture
优先放lane-local测试模块。

## 完成态

1. provider完成或失败后的terminal publication不会因buffer已满而永久阻塞；已接受item的顺序以及其后的End/Error
   语义保持确定，不能通过丢弃buffer或扩大channel容量掩盖问题。
2. runtime/request owner退出能在producer持有runtime/context clone时终止所有active stream；清理不能只依赖当前
   `Arc::strong_count == 1`偶然成立。producer必须观察取消并退出，registry、active task计数、lease与capability均
   exact-once归零，不形成runtime↔producer环。
3. server-stream/HTTP/file等外部边界遇到`StreamInternalItem`时仍fail closed，并在返回错误前cancel该stream。所有
   已取得stream后的异常退出（decode/coerce/callback error、early stop、request cancel）都必须通过统一consumer
   cleanup owner收口，正常End不重复cancel。
4. 正常callback for-in仍可消费pre-JSON internal carrier并调用真实owner executable；自然End、early break/cancel、
   source/runtime drop后旧carrier稳定返回`CapabilityExpired`，不得改变普通JSON item或pull-stream语义。
5. 动态测试用短timeout复现capacity=1关键时序：至少覆盖未消费stream+外层runtime drop且producer持有clone、外部
   response拒绝InternalItem、buffered item后End、buffered item后Error、重复cancel/drop；修复前测试应能稳定暴露
   泄漏/挂起，修复后task/stream/capability计数归零。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-host stream_runtime
cargo test -p skiff-runtime-host typed_execution_async_stream_cancel
cargo test -p skiff-runtime-eval in_process_stream
cargo test -p skiff-runtime-eval program_invocation
cargo test -p skiff-runtime-host typed_execution_callback_native
git diff --check
```

每个filter必须实际执行测试。另做反向搜索，确认没有用unbounded/增大capacity、detach泄漏task、TLS、router fallback
或生产test exemption规避blocker。

## 回报

提交一个clean commit，回报terminal状态机、owner/drop图、每个consumer异常出口的cleanup方式、复现测试在修复前后的
行为、命令与自验收矩阵。若保持End/Error顺序需要改变公共stream事件契约，先回报精确blocker，不自行扩公共语义。
