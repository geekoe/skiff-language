# P5-F445H-I6E4 time sleep current-scope consumer resume

状态：Ready。消费 E1 为 `NativeTimeCapability` 交付的 owned invocation control，使
`std.time.sleep` 按调用点 current scope挂起；其它同步time helper保持同步。

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

1. `sleep`开始时从E1 owned control读取current scope，取得其signals、absolute deadline与clock。
2. requested duration只决定normal wake，不是第二个scope deadline。
3. 一次真实sleep future与scope lease竞争；scope胜出drop sleep，normal wake先提交则不被同刻signal覆盖。
4. paused clock下derived deadline和ancestor/internal stop必须立即唤醒，不再每10ms轮询旧execution
   budget或request root snapshot。
5. 零时长、decode/clamp、date/time同步helper保持既有同步行为；不新增语言`yield`。
6. terminal后lease/timer/waiter归零；E4 post-await checkpoint继续投影精确owner。

## 唯一写集

```text
runtime/native/src/dispatch/time.rs
runtime/eval/src/program_execution/execution_scope_tests.rs
```

不得修改E1的 `NativeTimeCapability` getter、`RuntimeNativeInvocation`、artifact/std、其它native
dispatch或Cargo/lockfile。

## 测试

必须建立真实RED/GREEN，并覆盖normal wake、current/outer deadline、ancestor/internal stop、零时长、
同步helper不挂起、owner归零。

```text
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --list
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --nocapture
cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --list
cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --nocapture
cargo check -p skiff-runtime-native -p skiff-runtime-eval --locked
cargo fmt --check
git diff --check
```

两个selector listing均非零；Eval receipt必须从native projection走到真实sleep pending，不能只测getter。

## 停止与禁止

若需要修改 `RuntimeNativeInvocation`、artifact/std、把同步helper改成Pending/yield、增加轮询或E1共享
接口，提交 `TASK_SCOPE_EXPANDED` result并停止。禁止full gate、stable/live/network/Mongo、
merge/rebase/push。

## 完成

分开提交implementation/tests与
`P5-F445H-I6E4-time-current-scope-resume-result.md`。result给出commit/tree、RED/GREEN计数、paused-clock
矩阵、同步helper反查、实际写集，并标明 `I6_TIME_COMPLETE = YES/NO`。worktree保持clean。
