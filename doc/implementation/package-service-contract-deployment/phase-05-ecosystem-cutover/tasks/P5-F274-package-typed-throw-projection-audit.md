# P5-F274 Package typed-throw projection audit

状态：Ready（只读审计）。

## 直接父节点与权威链

- 直接父节点：
  `P5-F266-inline-test-effects-language-result.md`
- 父任务：
  `P5-F266-inline-test-effects-language.md`
- 唯一语义事实源：
  `doc/reference/testing.md` 第 7 节

父节点已确认：service contract signature 可以携带声明的 typed throw；Package callable
artifact signature 当前总是发布空 `throw_types`，所以即使实现会抛出声明错误，Package target
也无法在 inline effect 中使用 typed `throw`。这是 Package signature projection 缺口，不能
通过 effect DSL fallback 或放宽类型检查解决。

## DAG 位置

- 节点：确认 Package callable 的 declared throw 从 source 到 PackageArtifact、consumer
  signature、effect type-check 和 runtime linked type plan 的完整生产路径。
- 前置：F266/F267 已合流；审计基线为
  `codex/package-service-phase-05@f69fb5c`。
- 后续：审计 result 将作为独立实现任务的直接父节点；在实现合流前，F270 最终验收仍被此
  已知缺口阻塞。
- 当前成熟度：实现检查点，不是稳定候选。

## 已知入口与遮挡

- 已知空值写入点：
  `compiler/projection-input/src/package_callable_signatures.rs`。
- Package artifact 已有 `PackageCallableSignature.throw_types`，normalization 和 boundary
  代码也已有消费路径；不得据此假设 source owner、错误集合或 consumer handoff 已完整。
- 真实最小入口必须是 source 中声明/抛出 nominal error 的 Package callable，经普通
  Package publish/import 后，在 test service 的 inline Package effect 中使用同一 typed
  `throw`，并由测试代码 `catch<T>`。
- 上游空 `throw_types` 会使 effect type-check 提前失败，遮挡 runtime materialization。

## 本次只读范围

1. 沿 source declaration、lowering、projection input、PackageArtifact、artifact ingest、
   consumer signature、test-effect checking、link/runtime type plan 逐跳列出 owner。
2. 确认语言中“函数声明的可抛类型”的唯一事实来源；不得从函数体扫描或推断未声明错误。
3. 确认 public/local nominal error、跨 Package nominal error、alias 展开、零错误和多错误
   的正确表示及 fail-closed 条件。
4. 搜索生产生态中实际 Package callable typed throws 与测试替身需求，区分当前 blocker 和
   必要通用矩阵。
5. 给出一个有界实现任务的精确写入文件、非目标、快速测试、真实正例和关键负例。

## 禁止事项

- 不修改或提交代码、文档、fixture。
- 不改变 typed throw、catch、effect DSL 或 Package ABI 的公共语义。
- 不为历史 artifact 增加兼容。
- 不把 service contract 的 error owner 复制成第二套 Package 机制。
- 不操作 stable、不访问外网、不 push。

## 完成标准

提交一份可落盘的审计结论，至少包含：

- 每个生产跳点及当前精确缺口；
- 应修改与明确不应修改的 owner；
- artifact/wire/build identity 是否变化；
- 最小正负测试矩阵与真实端到端探针；
- 是否存在必须由用户决定的设计问题。

若 5 分钟内无法确认任务可执行，返回 `TASK_NOT_EXECUTABLE`、缺失事实和最小前置，不继续
扩张调查。

