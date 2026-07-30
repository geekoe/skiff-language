# P5-F179A：Actor Declaration Runtime Handoff

状态：Ready

## 直接父任务

- `P5-F178-compiler-actor-nominal-handle-result.md`

## 当前断点

`ActorDeclarationIr/ActorAbiIdentity`已建立，但尚未进入File IR/linked program。registry intrinsic call
只有T0，Runtime metadata无法证明id/bootstrap shape与actor ABI。不得从显示type/schema identity猜测，
也不得在每个call复制完整Actor声明。

## 范围

修改compiler lowering/emission、artifact-model File IR、artifact identity、runtime linker/linked-program及
聚焦测试，并写result。不得修改std、native dispatch、Router store/protocol或actor executor。

## 必须实现

- File IR按声明owner携带canonical `ActorDeclarationIr`，作为actor ABI/field bootstrap shape唯一事实。
- compiler lowering从typed actor declaration生成该事实，补齐registry intrinsic的T0 actor、T1 id、
  T2 bootstrap类型参数/精确引用；find/remove只携带其真实所需参数，不伪造bootstrap。
- File IR identity覆盖actor declaration及actor ABI identity；声明变化必须改变相关artifact identity。
- linker按call的T0名义type精确解析同一program中的ActorDeclarationIr，校验id/bootstrap type args，
  投影`RuntimeActorNativeMetadata`所需actor ABI、id encoding和field/bootstrap encoding事实。
- call不复制fields/method表；缺声明、错误owner、ABI/id/bootstrap错配fail closed。
- linked program保留actor declaration与implementation owner，供后续executor使用。

## 验证

- compiler lowering/emission、artifact identity、linker/linked-program聚焦测试；
- actor字段/id变化identity变化、无关actor不影响call target、缺失/错配拒绝；
- `cargo check --workspace`；
- `git diff --check`；
- 独立提交并写result。
