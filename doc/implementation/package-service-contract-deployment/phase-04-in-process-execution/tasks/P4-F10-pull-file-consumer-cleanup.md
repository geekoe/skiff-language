# P4-F10：Pull Error / File Consumer Cleanup

## Blocker、输入与边界

R02在exact clean candidate `484cab0bb42840e1031f05d0e0d48b7458695bef`第三次复验FAIL。channel callback lane及
F09 scope/owner/terminal主链已PASS，剩余两个terminal出口：

1. concrete pull source返回`Err`时`next_with_cancellation`用`?`直接传播，未从registry移除stream；active计数、source
   与lease继续存活且stream可再次poll。
2. `std.file.createFromStream`只在`InternalItem`拒绝时cancel。普通item之后的typed decode/encode/bytes coercion或
   file write early error没有覆盖整个native消费过程的cleanup owner；错误若被用户捕获，producer会等到request/root
   drop才清理。

权威输入为架构§6–§8、§12、§14，P4-F09、R02合同及R02@`484cab0` verdict。只修pull terminal transition、共享
consumer cleanup primitive与file native consumer动态证据；不得改变contract projection、channel terminal顺序、
scope identity、router/compiler或Wave 3 owner。

- 依赖：R02@`484cab0` FAIL。
- 解锁：R02 retry。
- branch：`codex/p4-f10-pull-file-consumer-cleanup`。
- worktree：`/Users/geek/workspace/skiff-p4-f10-pull-file-cleanup`。
- integration边界：只提交task branch，不merge integration/main、不push。

## 写入范围

独占concrete stream runtime pull branch/测试、`runtime/capability-context`共享consumer cleanup primitive、eval现有
cleanup delegate迁移、`runtime/native`与host file source adapter的`std.file.createFromStream`最小接线及lane-local
测试。不得把第二套cleanup owner复制进native；F09 eval guard应下沉或委托唯一共享owner。

## 完成态

1. pull source的`Ok(None)`、`Err(Cancelled)`与其它`Err`都执行稳定、exact-once terminal transition；source error立即
   移除registry、归零active/scoped计数、drop source/lifetime，后续poll稳定失败，不允许同一source再次产值。
2. 共享`StreamConsumerCleanup`从取得stream覆盖到整个消费操作结束；只有自然`StreamPoll::End` disarm。所有其它
   返回（pull error、typed decode/encode/bytes coercion、file open/write/flush/commit error、用户取消/early exit）
   同步cancel，重复runtime terminal/cancel保持幂等。
3. eval for-in/server/HTTP/drain继续消费同一共享cleanup primitive，不出现eval/native两套状态机。file
   `InternalItem`仍fail closed；普通item和pull stream公共语义不变。
4. 动态测试必须直接覆盖：pull source error后registry/active/lease归零且不可再次poll；file item decode/bytes错误；
   file write错误；每条都断言producer cancel/registry归零且正常End不额外cancel。测试不能只依赖最终request scope
   drop，必须在错误返回点立即观察清理。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-host stream_runtime
cargo test -p skiff-runtime-native create_from_stream
cargo test -p skiff-runtime-eval program_invocation
cargo test -p skiff-runtime-host typed_execution_async_stream_cancel
cargo test -p skiff-runtime-host typed_execution_callback_native
git diff --check
```

若native filter名称无匹配，选择实际最窄非零filter并回报，不得以0 tests作为证据。另反向搜索所有
`next_file_source_stream_item`/pull source error/`StreamConsumerCleanup`生产引用，确认唯一cleanup owner、无错误出口漏网，
且无buffer扩容、unbounded、TLS/task-local、router fallback或test exemption。

## 回报

提交一个clean commit，回报pull terminal状态转换、共享guard owner/marker生命周期、file每个异常出口、修复前后动态
证据、命令与自验收矩阵。若file capability API无法把guard覆盖到write完成，回报精确trait/lifetime blocker，不得只在
item decode处局部cancel冒充完成。
