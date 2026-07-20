# P4-F14R：Supervised Stream Consumption Handoff

## Blocker、输入与边界

F14在`172d6e8`上把prepared producer的注册、next/drain/cancel统一到provider/context的同一`StreamRuntime`后，
既有`runtime_program_create_from_stream_prefers_producer_error_after_consumer_error`仍返回consumer的serviceDb错误。
根因是F10共享`StreamConsumerCleanup`在native consumer返回错误前同步cancel/remove同一registry；outer composite owner
随后无法drain尚未运行或已pending的producer typed terminal。恢复双handle、yield竞态、把unknown映射成producer错误或
削弱standalone cleanup都违反既定语义。

权威输入为架构§6–§8、§12、§14，P4-T05、F09、F10、F14、R02与T10合同，以及F14独立范围分类。该分类确认这是
设计内的implementation ownership扩张，不改变公共契约：prepared producer复合调用必须显式、唯一、exact-once地
协调child consumer cleanup与outer drain；普通非supervised consumer仍在错误点立即cancel。

- 依赖：F14 `TASK_NOT_EXECUTABLE`范围报告与独立分类PASS。
- 解锁：F15与T10 retry。
- branch：继续`codex/p4-f14-stream-runtime-owner`，允许审阅并采用其中未提交的same-handle诊断patch。
- worktree：`/Users/geek/workspace/skiff-p4-f14-stream-owner`。
- integration边界：提交一个完整F14R commit，不merge integration/main、不push。

## 写入范围

独占：

- `runtime/eval/src/program_stream.rs`及最窄driver/eval回归测试；
- `runtime/capability-context/src/stream_cleanup.rs`与必要`lib.rs`导出/primitive tests；
- `runtime/eval/src/{eval_context,native_capability,capabilities}.rs`中prepared file-source projection的最小delegate接线。

优先在eval file-source wrapper记录typed End/ProducerError，避免修改native trait、host registry和
`runtime/native/src/dispatch/file.rs`。若上述范围无法观察typed terminal，先报告精确缺口，不得自行扩大到第二套native/
host状态机。不得持有Interpreter、task、activation owner或反向Runtime `Arc`形成owner环。

## 完成态

1. 演进现有`StreamConsumerCleanup`/EndMarker为唯一supervised consumption lease/guard，不复制registry或lifecycle
   state machine。standalone guard非End退出继续同步hard-cancel并立即清零；supervised child非End退出只同步记录
   cleanup request并把obligation交回outer owner，不移除producer terminal。
2. prepared producer显式持有创建channel的同一`StreamRuntime`与唯一drain obligation。outer在对外返回前claim并用
   该handle drain：producer typed Error覆盖consumer error；producer End保留consumer error；request/outer cancel、
   drop、panic直接hard-cancel。
3. native/file-source wrapper用typed marker记录真实End/ProducerError；删除对`unknown Stream value`/
   `already consumed`字符串的终态推断。wrong handle/unknown继续typed Decode/fail closed。
4. shared typed coordination至少区分open、observed End、observed ProducerError、cleanup requested和finalized；最终释放仍
   委托现有concrete `StreamState::finish` CAS，cancel callback、registry、active count、request lease均exact-once。
5. nested producer preparation/clone/type error、deferred conversion、success/cancel/drain的每个实际producer都使用自己的
   handle/obligation；`program_stream.rs`只保留execution/orchestration，协调状态留在共享primitive。

## 唯一验证 ownership

```bash
cargo test -p runtime runtime_program_create_from_stream_prefers_producer_error_after_consumer_error
cargo test -p skiff-runtime-native create_from_stream
cargo test -p skiff-runtime-capability-context stream_cleanup
cargo test -p skiff-runtime-eval program_stream
cargo test -p skiff-runtime-host stream_runtime
cargo test -p skiff-runtime-host typed_execution_async_stream_cancel
git diff --check
```

每个filter必须非空或替换成实际最窄非零filter并回报。新增确定性barrier测试覆盖consumer先失败、producer尚未运行；
另覆盖consumer error+End、consumer error+typed Error、native已观察Error不二次drain、commit error after End cancel一次、
natural End零额外cancel、standalone decode/write error立即清零、outer cancel/drop清零、nested early error exact-once及
unknown/wrong registry fail closed。不得运行完整runtime gate。

## 回报

提交一个clean commit，回报same-handle与obligation状态转换、standalone/supervised差异、typed terminal记录点、
exact-once owner、旧文本推断删除、修复前后证据、自验收矩阵和extra-review。若必须改变native/host公共trait或第二套
registry，报告`TASK_NOT_EXECUTABLE`，不得自行扩张。
