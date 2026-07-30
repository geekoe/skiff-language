# P5-F178B：Runtime Builtin Descriptor Cleanup

状态：Ready

## 直接父任务

- `P5-F178A-actor-declaration-artifact-checkpoint-result.md`

## 范围

修改runtime各crate中因F177删除`TypeDescriptorIr::Native`、硬切`TypeRefIr::Builtin`而残留的
consumer及直接fixture。不得实现actor registry、actor executor、compiler语义或std surface。

## 必须实现

- 删除runtime linker/model/eval/host中的native descriptor match和fallback。
- builtin type ref继续按各具体builtin现有layout/materialization处理，不引入opaque generic native
  fallback。
- 测试fixture改用Builtin或真实structured descriptor；不得恢复legacy wire兼容。
- Runtime内部ActorRef/control DTO保持原样，本任务不删除平台内部actor能力。

## 验证

- 受影响runtime crate聚焦测试；
- runtime内`TypeDescriptorIr::Native`无命中，`TypeRefIr::Native`无命中；
- `cargo check --workspace`首错越过机械runtime consumer；
- `git diff --check`；
- 独立提交并写result。
