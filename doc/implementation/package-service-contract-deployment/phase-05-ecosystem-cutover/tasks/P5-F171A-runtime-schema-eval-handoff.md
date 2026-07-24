# P5-F171A：Runtime Schema Eval Handoff

状态：Ready

## 直接父任务

- `P5-F166-runtime-package-schema-hydration-result.md`

## 当前断点

loader的`ServiceContractStore`已经按contract保存不可变`ResolvedServiceSchema`，但Host构造
`ActiveAssemblyContextSet`时只保留`ServiceContract`。`RuntimeAssemblyEvalResolver`及
`RuntimeAssemblyServiceCallTarget`没有schema入口，导致普通、stream和WebSocket执行路径无法消费
admission结果。

## 范围

修改`runtime/host`的active assembly context、`runtime/eval`的assembly seam及其直接fixture/tests，
并写result。不得修改具体boundary materialization、stream、callback或WebSocket执行逻辑。

## 必须实现

- Host从已hydrated candidate/store取得每个`ServiceContractRef`对应的
  `Arc<ResolvedServiceSchema>`，与contract一起固定在同一assembly generation的immutable context。
- `RuntimeAssemblyEvalResolver`提供按精确`ServiceContractRef`获取admitted schema的typed接口。
- ingress与internal service call构造`RuntimeAssemblyServiceCallTarget`时都绑定同一
  `Arc<ResolvedServiceSchema>`，target提供只读accessor。
- 校验contract ref、schema contract identity及generation assembly owner一致；缺失或错配在target
  构造时fail closed。
- 不得传Package index、artifact root、resolver或文件系统能力进入eval；不得复制records或重新
  admission。
- 保持现有activation、operation target和request generation语义。

## 验证

- `cargo test -p skiff-runtime-eval assembly_seam`及Host active-context聚焦测试；
- ingress/internal call同schema Arc、缺失、错配及generation隔离覆盖；
- `cargo check -p skiff-runtime-host`；
- `git diff --check`；
- 独立提交并写`P5-F171A-runtime-schema-eval-handoff-result.md`。
