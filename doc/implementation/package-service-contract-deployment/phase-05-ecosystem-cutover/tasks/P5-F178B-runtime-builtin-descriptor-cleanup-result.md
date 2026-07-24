# P5-F178B：Runtime Builtin Descriptor Cleanup Result

状态：Completed

## 直接父任务

- `P5-F178B-runtime-builtin-descriptor-cleanup.md`

## 交付

- linker删除已不存在的`TypeDescriptorIr::Native`转换分支，linked program同时删除内部
  `LinkedTypeDescriptor::Native`模型及其序列化、类型引用遍历和执行期消费分支。
- HTTP类型规划不再通过native symbol fallback识别官方HTTP类型，只接受正确package identity下的
  真实声明名称。
- native调用签名中的builtin继续映射到既有具体layout/materialization；未知builtin现在以
  `InvalidArtifact`失败，不再生成opaque `Unknown`计划，并新增聚焦测试固定该行为。
- eval catch类型和code linker删除native descriptor残留分支。
- 未修改Runtime内部`ActorRef`、control DTO，也未实现actor registry、executor或compiler语义。

## 验证

通过：

```text
cargo test --locked \
  -p skiff-runtime-linked-program \
  -p skiff-runtime-linker \
  -p skiff-runtime-linked-type-plan \
  -p skiff-runtime-eval
# eval: 84 passed
# linked-program: 18 passed
# linked-type-plan: 7 passed
# linker: 17 passed

cargo test --locked -p skiff-runtime-linked-type-plan
# 8 passed（含新增unknown builtin fail-closed测试）

cargo check --locked --workspace
# PASS

rg "TypeDescriptorIr::Native|TypeRefIr::Native|LinkedTypeDescriptor::Native|native_builtin_fallback_plan" \
  runtime -g '*.rs'
# zero matches

git diff --check
# PASS
```
