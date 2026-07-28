# P5-F445H-E4R8 ordinary service-error consumer stack preflight

状态：Ready。E4R6 完整 lib重验暴露的第二个默认线程栈 blocker只读探查。目标是定位
ordinary service-error consumer的精确future边界并冻结修复；本任务不修代码。

## 直接父节点与冻结事实

- `P5-F445H-E4R6-callback-stack-shape-closure-result.md`
- `P5-F445H-E4R6-callback-full-suite-stack-preflight-result.md`
- `P5-F445H-E4R5-combined-integration-acceptance-result.md`

当前候选 commit为 `464a3319b153527d5d33093d52ea6af97b6f997b`。callback call-site boxing已
关闭其栈形状blocker。串行完整lib继续到以下test时再次 stack overflow / `SIGABRT`：

```text
assembly_execution::ordinary::tests::service_error_consumer::
ordinary_exact_public_and_internal_catches_hit_while_unlinked_catch_misses
```

当前没有backtrace、阈值或focused证据，不能假设它与callback同一修复。

## 角色与唯一写集

唯一允许写入：

- 新增 `P5-F445H-E4R8-service-error-consumer-stack-preflight-result.md`

production、tests、fixture、Cargo/manifest/lockfile及其它文档只读。临时日志只放ignored
`build/`。不得patch源码、直接修复或派子 Agent。

## 必须回答

1. exact test单独、`--test-threads=1`是否稳定复现；
2. Ready/Pending、public/internal/unlinked catch哪个子路径造成stack峰值；
3. 栈阈值是否为有限async state放大，还是有真实递归/fixture循环；
4. static调用链从test到 ordinary evaluator、service error import、catch projection的精确路径；
5. 过大的concrete future在哪个private call-site进入generic evaluator链；
6. 唯一修复owner、最小写集和不可改变的error/catch语义；
7. RED/GREEN和完整lib重验要求。

需要特别检查：

- 当前test是否在一个future中串行组合多个超大case；
- service call prepared wait/finalize是否已由J1证明owned；
- error import、exact public/internal catch与unlinked miss是否可能递归；
-一个private boxing边界能否解决，还是必须拆test/修改公共owner。

不得把增大 `RUST_MIN_STACK`、改test attribute、ignore/拆断言当production修复。

## 有界诊断

使用独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-e4r8-preflight/build/cargo-target
```

依次：

1. exact test默认与 `--test-threads=1`；
2. 如有相邻较小test，做Ready/Pending或单case对照；
3. 有限 `RUST_MIN_STACK`阈值对照，只用于定位；
4. 只读type-size/static call-chain诊断；可用一次no-run type-size编译；
5. 不运行完整lib/eval。

若test本身一次组合多个case而无法无修改隔离，应从源码结构、栈阈值和async type尺寸形成证据，
或返回需要单独instrumentation节点；不得虚构具体frame。

不得运行stable、live、network、MongoDB或其它仓库。

## Result要求

状态只能为：

- `READY_FOR_E4R8_FIX`：单一原因、owner、写集和可执行验收已冻结；
- `TASK_SCOPE_EXPANDED`：需要公共owner/多个独立节点；
- `TASK_NOT_EXECUTABLE`：有界探查无法安全定位。

result记录：

- commit/tree、clean状态；
- exact/线程/栈阈值矩阵；
-调用链与相关async state尺寸；
-被排除的递归、并发、fixture假设；
-唯一owner、允许写集、禁止面；
-保持的public/internal/unlinked catch语义；
- RED/GREEN和一次完整lib重验要求；
-是否需要用户决策。

发现明显小修也只能写入result，不能实现。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r8-preflight
branch   codex/p5-f445h-e4r8-preflight
```

只提交result；返回commit、状态、根因证据、修复写集和clean worktree。不得
merge/rebase/push。

风险：高。必须区分test组合栈形状与真实service-error/catch递归。
