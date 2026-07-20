# P4-F14：Stream Runtime Owner Consistency

## Blocker、输入与边界

T10在exact clean candidate `453c11f0c809cf0af6788375988eb973237e5aaa`运行runtime gate时，既有
`runtime_program_create_from_stream_prefers_producer_error_after_consumer_error`从main的PASS回归为先返回
`serviceDb is not configured`。Phase async/stream改动让`prepare_stream_producer`在provider context的
`stream_runtime`注册channel，但success/cancel/drain仍访问interpreter的`self.stream_runtime`；两个handle不同时，
drain查错registry并吞掉更具体的producer typed-decode error。

权威输入为架构§6–§8、§12、§14，P4-T05、F08–F10、R02与T10合同。只修stream producer准备、消费、drain、cancel
全过程的显式handle owner一致性；不得重新共享activation mutable owner、引入TLS/global registry或改变error precedence。

- 依赖：T10@`453c11f` runtime gate FAIL，且main@`0cbf533`同一测试PASS。
- 解锁：T10 retry。
- branch：`codex/p4-f14-stream-runtime-owner`。
- worktree：`/Users/geek/workspace/skiff-p4-f14-stream-owner`。
- integration边界：只提交task branch，不merge integration/main、不push。

## 写入范围与完成态

独占`runtime/eval/src/program_stream.rs`及最窄相关eval/driver测试。不得修改host fixture、capability-context公共状态机、
native file semantics或router。

1. prepared producer显式携带唯一provider/context `StreamRuntimeHandle`；注册、next/drain、success、cancel与cleanup都使用
   同一handle，不在中途回落`self.stream_runtime`。
2. consumer error后仍drain producer，并保持producer error优先级；unknown/consumed只能表示真实terminal，不得由查错
   registry制造。
3. success、consumer error、producer error、cancel与drop均保持exact-once terminal/cleanup；provider与receiver
   activation context仍隔离。
4. 反向审计同一prepared stream生命周期的所有`self.stream_runtime`访问，消除owner split并补最小回归测试。

## 唯一验证 ownership

```bash
cargo test -p runtime runtime_program_create_from_stream_prefers_producer_error_after_consumer_error
cargo test -p skiff-runtime-eval program_stream
cargo test -p skiff-runtime-eval in_process_stream
cargo test -p skiff-runtime-host typed_execution_async_stream_cancel
git diff --check
```

若filter无匹配，选实际最窄非零filter并回报。不得运行完整runtime gate。

## 回报

提交一个clean commit，回报handle ownership前后图、error precedence、terminal/cleanup反向搜索、修复前后测试与
自验收矩阵。若修复需改变公共stream/capability API，立即报告范围扩张，不得在本任务自行决定。
