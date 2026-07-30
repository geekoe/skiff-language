# P5-F171E：Eval Callback Shared Schema Records

状态：Ready

## 直接父任务

- `P5-F171D-native-callback-shared-schema-records-result.md`

## 当前断点

native callback已使用shared records，但eval `callback_native`内部schema closure辅助函数仍要求
value-owned record map，无法调用新的boundary/native接口。

## 范围

只修改`runtime/eval/src/assembly_execution/callback_native.rs`及其直接测试，并写result。

## 必须实现

- callback closure/descriptor lookup统一借用shared `Arc<Record>` map。
- 不复制record/descriptor；保持完整Package identity、cycle和缺record拒绝语义。
- 删除该文件所有value-owned Package schema map签名。

## 验证

- callback聚焦测试（若仍受F171/F172编译阻断，记录首错必须越过本文件）；
- 目标文件旧owned map反搜无命中；
- `git diff --check`；
- 独立提交并写result。
