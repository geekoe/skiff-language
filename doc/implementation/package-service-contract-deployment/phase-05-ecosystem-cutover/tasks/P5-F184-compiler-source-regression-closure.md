# P5-F184：Compiler Source 与 Lowering 回归闭合

状态：Ready

## 直接父任务

- `P5-F178-compiler-actor-nominal-handle-result.md`

## 问题

当前 integration 的 `skiff-compiler-source --lib` 全量测试有 13 项失败，并进一步导致
compiler-lowering、compiler integration 和 Router compiler fixture 测试失败。已观察到的类别包括：

- Actor 名义类型/方法事实；
- 普通与泛型 impl receiver 字段解析；
- interface/callable effects；
- std source 中 receiver 方法类型事实。

## 目标

逐项判断是 production 回归还是旧断言，修复 canonical source type/effect facts，使 Actor 与普通
Package 类型共存，而不恢复 ActorRef、native type、动态 receiver 或未知调用 fallback。

## 范围

- `compiler/source`
- `compiler/lowering`
- 必要的直接 compiler 聚焦测试/fixture

不得修改 Runtime、Router、Package schema store 或放宽 contract/boundary 的失败关闭语义。

## 必须实现

- 先记录 13 项失败的精确分类与首个错误 owner；
- Actor receiver 仍生成专用 ActorMethod target；
- 普通 record/generic impl receiver 的字段与方法继续使用静态类型事实；
- interface/callable effects 不得因 Actor 分支变成 unknown；
- std 源码中的 Duration/HTTP 等普通 Package 类型 receiver 能静态解析；
- 若测试断言已被权威设计替代，只更新到新 canonical 结果，并给出正负探针；
- 不允许通过跳过测试、默认 unknown、动态 dispatch 或 source 名字猜测来通过。

## 验证

- `cargo test -p skiff-compiler-source --lib` 全通过；
- `cargo test -p skiff-compiler-lowering --lib` 全通过；
- 受影响的 compiler integration/Router compiler fixture 聚焦测试；
- ActorMethod、普通 impl、泛型 impl、interface effects、std receiver 正负探针；
- `cargo check --workspace`、`git diff --check`；
- 独立提交并写 `P5-F184-compiler-source-regression-closure-result.md`。

