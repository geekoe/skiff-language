# P5-F445H-I6E4R prepared-time fixture closure

状态：Ready。I6E4 的 current-scope sleep实现与新selector已通过，但 E1 机械增加owned-control getter后的
既有 `PreparedTestExecutionControl` 没有提供 `ExecutionScope`，使一条 prepared-time测试在首次
poll非零sleep时返回 `InvalidArtifact`。本节点只修复该测试fixture并重验time闭环。

## 直接父节点

- `P5-F445H-I6E4-time-current-scope-resume-result.md`
- `P5-F445H-I6E1-shared-carrier-delivery-checkpoint-result.md`

## 固定输入

```text
I6E4 implementation commit  0f250dff41ec91a06c89a4716b029d69e6edc116
I6E4 implementation tree    a7a0065dfd6b9911025fe96db0e4aac23e377fa7
integration base commit     b6d0ec5e9171e38730720904f8a21b3946459d1c
integration base tree       a1200dbf921b8add778d92246deb48f83647ac42
```

## 唯一修改

```text
runtime/native/src/dispatch/prepared_tests.rs
```

让 `PreparedTestExecutionControl` 使用与测试既有request root/clock一致的真实
`ExecutionScope`，使E1的owned-control getter与I6E4 sleep首次poll可以取得current scope。

要求：

1. 不为测试绕过scope读取，不把非零sleep改回轮询。
2. 不改变“prepared wait不借用caller heap并观察actual Pending”的原断言。
3. fixture terminal后lease/timer/waiter归零；不得使用全局或task-local side channel。
4. 不修改production、其它tests、Cargo/lockfile。

## RED / GREEN

先原样记录既有失败：

```text
cargo test -p skiff-runtime-native \
  prepared_time_wait_does_not_borrow_caller_heap_and_observes_actual_pending -- --nocapture
```

修复后运行：

```text
cargo test -p skiff-runtime-native \
  prepared_time_wait_does_not_borrow_caller_heap_and_observes_actual_pending -- --nocapture
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --list
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --nocapture
cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --list
cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --nocapture
cargo check -p skiff-runtime-native -p skiff-runtime-eval --locked
cargo fmt --check
git diff --check
```

两个selector listing必须非零；原既有测试必须1/1通过。

## 停止与完成

若需要修改production、E1 shared API、I6E4 sleep行为或其它fixture，提交 `TASK_SCOPE_EXPANDED`
result并停止。禁止full gate、stable/live/network/Mongo、merge/rebase/push。

分开提交fixture与
`P5-F445H-I6E4R-time-prepared-fixture-closure-result.md`。result给出commit/tree、RED/GREEN、selector
计数、实际写集和 `I6_TIME_COMPLETE = YES/NO`。worktree保持clean。
