# P1-T03：Typed Effect Semantic Leaf

## 目标

删除`Empty`、placeholder和raw effect metadata对“未分析”的含混表达，建立从source到artifact可完整
保留、默认fail-closed的typed callable effect leaf。本阶段不实现完整effect分析。

## 依赖与 worktree

- 无前置代码任务；可与T01/T04从文档checkpoint并行。
- 建议branch：`codex/package-service-p1-t03-typed-effect-leaf`。

## 权威数据形状

```text
CallableEffectSummary
  = Unknown { reason: AnalysisPending }
  | Analyzed { effects: CallableMayEffects }

CallableMayEffects
  writesCallerReachable: bool
  returnsCallerAlias: bool
  throwsCallerAlias: bool
  escapesCallerValue: bool
  requiresSameHeapIdentity: bool
  invokesUnknownTarget: bool
  maySuspend: bool
```

字段全部显式；没有serde default把缺字段解释为false。artifact map以稳定operation ABI id为key；source
阶段可以SourceSymbolKey作为owner-local key，但进入compiled/projection handoff时必须显式映射，不能用
display path猜测。

## 完成态

1. artifact-model提供上述typed DTO并使用tagged、deny-unknown wire shape。
2. `ConfigAndEffectMetadata`拆成职责明确的config facts与callable effect facts；禁止继续使用一个generic
   metadata对象同时冒充两者。PackageUnit/package-test相关字段同步改成明确结构。
3. `SourceConfigAndEffectMetadata`拆分；`SourceEffectMetadata::Empty`删除。source/compiled handoff为每个进入
   public callable surface的operation保留`Unknown(AnalysisPending)`。
4. projection/emission只映射typed facts；`projection/runtime/operation_effects.rs`的placeholder删除。
5. 现有runtime行为不从Unknown推导任何权限或优化。未来boundary consumer遇到Unknown必须拒绝；本阶段
   以单元测试锁定该helper语义。
6. reason code进入semantic bytes；自由诊断detail不属于DTO也不进入identity。

## 写入范围

- artifact-model package/effect/config leaf与tests。
- compiler source config/effect owner、compiled/projection-input DTO、直接projection/emission consumers。
- compiler package-test producer/test support中仅与新typed shape直接相关的构造。T03负责让现有compiler
  production/package-test路径在本checkpoint端到端使用新wire；T06只在此基础上收敛重复builder。

不要实现AST fixed-point、boundary projection、ServiceContract或package identity hash；T05负责identity。

## 验证

```bash
cargo fmt --all -- --check
cargo test -p skiff-artifact-model
cargo test -p skiff-compiler-source -p skiff-compiler-compiled -p skiff-compiler-projection-input
cargo test -p skiff-compiler-projection -p skiff-compiler-emission
git diff --check
```

测试必须覆盖Unknown round-trip、缺tag/字段拒绝、未知字段拒绝、所有public callable都有entry、映射不依赖
display path、normal return alias与throw/error payload alias可独立表达、Unknown不能被boundary helper视为
available。

## 自验收与回报

反向搜索`SourceEffectMetadata::Empty`、旧`EffectMetadata`、`precision: "placeholder"`和空effects map的
production构造；每个剩余命中必须是迁移fixture或明确测试。提交自验收矩阵和commit。
