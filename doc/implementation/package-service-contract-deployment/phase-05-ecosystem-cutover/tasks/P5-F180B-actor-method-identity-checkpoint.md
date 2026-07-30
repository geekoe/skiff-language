# P5-F180B：Actor Method Identity Checkpoint

状态：Ready

## 直接父任务

- `P5-F180A-actor-executor-gap-audit-result.md`

## 范围

修改artifact-model/identity、compiler typed actor method facts、lowering identity producer、
linked-program/linker及聚焦测试。不得实现method transport、Router dispatcher或Runtime executor。

## 必须实现

- `ActorPublicMethodIr`真实生成并包含稳定method identity、参数、返回、`maySuspend`与公开ABI事实。
- Actor ABI identity覆盖id type、字段、公开方法签名及actor runtime ABI。
- Actor implementation identity独立于整个request/package build，覆盖规范化actor方法IR及其可达
  依赖；无关Actor/不可达代码不影响。
- 建立`ResolvedCallTarget::ActorMethod`、`CallTargetIr::ActorMethod`与linked actor dispatch plan；
  Actor receiver不得落为LocalExecutable/ExternalServiceSymbol或普通ExecutableAddr。
- call只引用Actor declaration owner、actor ABI/implementation、method identity；不复制声明/方法表。
- Actor不进入TypeAddr/type table/record descriptor。

## 验证

- identity mutation matrix：id/field/signature/maySuspend/method IR/可达依赖变化；无关变化不变；
- 真实source Actor declaration+impl+caller产生专用call target并成功link；
- owner/method/ABI/implementation错配与普通direct target伪装拒绝；
- artifact/identity/compiler/linker聚焦测试；
- `cargo check --workspace`；
- `git diff --check`；
- 独立提交并写result。
