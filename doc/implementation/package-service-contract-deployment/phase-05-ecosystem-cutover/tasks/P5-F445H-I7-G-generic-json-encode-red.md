# P5-F445H-I7-G generic `std.json.encode` RED

## 1. 目标

在不修改 production 的前提下，以 hermetic 的 compiler → canonical artifact → linker →
Eval 路径验证 I7 M4 的 `unsupported native target std.json.encode` 是否由 generic
`std.json.encode<T>` 的 runtime type substitution / native plan 闭合缺口稳定造成。

## 2. 冻结输入

- Skiff commit：`5c0f8222972e4612224e0660e88e6054874ddd03`
- Skiff tree：`cf98566873d974a63a9759a2856ecc28efbde5a4`
- 权威声明：`std/json.skiff`
- shared native signature：`artifact-model/src/native_signature.rs`
- Eval native plan：`runtime/eval/src/native_invocation.rs`
- JSON dispatcher：`runtime/native/src/dispatch/json.rs`
- native plan fail-closed：`runtime/native/src/dispatch/invocation.rs`

## 3. 写集与禁令

只允许：

- `runtime/eval/src/assembly_execution/ordinary/tests.rs`
- `runtime/eval/src/assembly_execution/ordinary/tests/source_generic_json_encode_red.rs`
- 本任务与 result 文档

禁止修改 production、Internals、official packages；禁止 stable instance、Mongo、网络和 live
provider；禁止在本任务修复发现的 production 缺口。

## 4. RED 合同

测试必须从真实 Skiff source 经 compiler 生成 File IR，使用 canonical artifact store 与 runtime
linker 形成 Eval image，不得手写 native call target。fixture 固定：

```skiff
function encodeJson<T>(value: T) -> Json {
  return std.json.decode<Json>(std.json.encode<T>(value))
}
```

generic encode 至少覆盖：

1. package 内 private nominal record；
2. dependency package 的公开 nominal / exact package symbol；
3. nested container `Array<LocalPayload>`。

同一测试先证明 direct concrete `std.json.encode<string>` 为 GREEN，并证明 generic
`std.json.decode<T>` 为 GREEN。随后 generic encode 的目标语义断言在当前 production 上应 RED；
若首错不是精确 `unsupported native target std.json.encode`，不得把 M4 尾错归因于本 owner。

## 5. RED 命中后的 successor 冻结

result 必须记录：

- compiler File IR 的 wrapper type parameter 与 caller concrete T；
- linker 后 target / type argument 是否保持；
- 首个丢失 native plan 的 `require_plan` owner；
- 唯一 production owner 与最小写集；
- 修复后应转 GREEN 的同一 hermetic 测试。
