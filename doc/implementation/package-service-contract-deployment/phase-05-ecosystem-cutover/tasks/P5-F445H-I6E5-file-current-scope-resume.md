# P5-F445H-I6E5 file current-scope consumer resume

状态：Ready。消费 E1 已交付到 Host file adapter 的 owned control，使 file direct/provider/source
真实Pending按调用点current scope结束，并保留既有资源清理owner。

## 直接父节点

- `P5-F445H-I6E1-shared-carrier-delivery-checkpoint-result.md`
- `P5-F445H-I6D-host-operation-current-scope-result.md`
- `P5-F445H-I6E-invocation-carrier-delivery-preflight-result.md`

## 固定输入

```text
E1 implementation commit  ba66719e03cbabde2e159b94761cc1a1c71b35d2
E1 implementation tree    0b1972158d710c4355274f7fb272be292dcc7927
integration base commit   e942efa99460ea2b9bf29f07d8dfe855c9715aff
integration base tree     46abc10c8fbdab6e70f2ea071539382dbf03a1be
```

## 行为要求

1. 在 `file_stream` adapter建立一个共享的scoped lower-future owner，覆盖六个direct/provider操作和
   source `next`；每次操作开始时读取current scope。
2. lower future与scope lease竞争；scope胜出drop lower future，normal completion先提交则不被同刻
   signal覆盖。
3. current deadline、outer timeout、ancestor/internal stop均唤醒真实Pending；post-await checkpoint
   继续投影精确owner。
4. 不在adapter和store两层重复acquire lease；store/provider选择逻辑保持不变。
5. 已进入外部blob/DB/spawn-blocking effect后不承诺回滚；不得把未知副作用伪装成已取消。
6. `FileIngest` scope胜出时必须通过drop cleanup清理临时路径；既有`StagedFile` drop owner保留。
7. source late item/error不得在scope terminal后交付；lease/timer/waiter与临时资源归零。

## 唯一写集

Production：

```text
runtime/host/src/eval_capability_adapter/file_stream.rs
runtime/host/src/host/file_runtime.rs
```

Tests：

```text
runtime/host/src/eval_capability_adapter/file_stream_tests.rs
runtime/host/src/host/file_runtime/tests.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/file_create_from_stream.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/support.rs
```

不得修改 `store.rs`、`stream_runtime.rs`、E1共享接口、DB state machine、Cargo/lockfile。若实际不存在
某个列出的test module，允许在同目录按现有module结构新增；不得创造不存在的production
`file_stream.rs` owner。

## 测试

覆盖direct Ready/Pending、provider Pending、source Pending、current/outer deadline、ancestor/internal
stop、normal竞争、late completion、ingest temp cleanup、owner归零及create-from-stream纵向receipt。

```text
cargo test -p skiff-runtime-host f445h_i6_file_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_file_scope -- --nocapture
cargo test -p skiff-runtime-eval f445h_i6_file_projection_to_pending -- --list
cargo test -p skiff-runtime-eval f445h_i6_file_projection_to_pending -- --nocapture
cargo check -p skiff-runtime-host -p skiff-runtime-eval --locked
cargo fmt --check
git diff --check
```

两个selector listing均非零。

## 停止与禁止

若需要DB/blob回滚承诺、全局cleanup supervisor、把lease移到`store.rs`、修改E1共享接口或新增公开
cancel/lifecycle metadata，提交 `TASK_SCOPE_EXPANDED` result并停止。禁止full gate、stable/live、
network/Mongo、merge/rebase/push。

## 完成

分开提交implementation/tests与
`P5-F445H-I6E5-file-current-scope-resume-result.md`。result给出commit/tree、RED/GREEN、资源owner矩阵、
真实receipt、实际写集，并标明 `I6_FILE_COMPLETE = YES/NO`。worktree保持clean。
