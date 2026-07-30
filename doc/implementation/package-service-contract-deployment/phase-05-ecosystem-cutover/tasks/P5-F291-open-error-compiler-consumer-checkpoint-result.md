# P5-F291 Open error compiler consumer checkpoint result

状态：实现检查点已建立；解除 language/source/lowering consumer。

## Exact checkpoint

- integration commit：
  `c08077c2efb6826f8f9dbd802f211fe9d4106115`
- shared model：
  `P5-F284-open-error-model-acceptance-result.md`
- dependency callable field-read：
  `P5-F285-dependency-result-field-read-fix-result.md`
- std error surface：
  `P5-F287-std-error-surface-migration-result.md`
- artifact/contract consumers：
  `P5-F288-open-error-artifact-contract-consumers-result.md`
- open-channel effect consumer：
  `P5-F290-open-error-effect-consumer-result.md`

上述结果继续引用 F280 audit、F279 design result 与唯一权威架构
`doc/architecture/package-service-contract-deployment.md`。

## 当前共享接口

- File IR 已严格区分 record、representation、named union、alias、interface，并要求 named-union
  branch identity 输入、throw/call instruction site 和 required catch type。
- callable/operation DTO 已无 `throw_types` / `errors`；artifact/contract consumer 与 identity
  generation 已迁移。
- std 已无 `ErrorPayload`，并公开普通名义类型 `std.service.InternalError`。
- detached service call 的 return/throw provenance 均为 `Fresh`；F278 same-heap、alias、write、
  escape 与 unknown facts 保持独立。
- F285 owner-aware dependency callable signature rehydration 已合入，language consumer 必须保留。

## 当前首次遮挡与剩余 owner

聚焦 compiler consumer tests 在枚举前首先被
`compiler/core/src/type_closure/mod.rs` 的旧
`TypeDescriptorIr::Union { variants }` 匹配遮挡；新字段为 `branches`。

剩余 production consumer 集中在：

- `compiler/core/src/type_closure/**`
- `compiler/source/**`，排除已完成的 `callable_effects/**`
- `compiler/lowering/**`

已确认旧形状包括 declaration descriptor、optional catch、缺失 throw/call site、
source contract error set、test-effect declared throw set，以及相关 type traversal。它们共同遮挡
F288/F290 尚未执行的 compiler combined tests。Router generation、runtime identity/channel 与 wire
不属于本 checkpoint。

本 checkpoint 是可继续实现的基线，不是预验收或稳定候选。

