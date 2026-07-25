# P5-F221 Public LocalType boundary normalization result

## 结果

完成。

公开 callable ABI 中的 `LocalType` 现在会按当前模块和 `typeIndex` 反查
`FileIrUnit.declarations.types` 中的唯一声明身份，再从 package schema projection 已生成的
`(module, symbol) -> PackageSchema` 映射取得规范引用。边界投影不会再展开该名义类型的结构。

归一化同时用于本地类型闭包验证和最终 `ContractTypeRef` 投影，因此参数、返回值以及
Array/Map/Nullable 等递归容器中的公开本地类型得到相同处理。

## 失败关闭

以下情况继续返回 `UnsupportedBoundaryType`：

- 模块不存在或存在重复 File IR unit；
- `typeIndex` 没有唯一声明；
- 类型表缺项，或声明名与对应类型表项不一致；
- 声明没有出现在公开 package schema 映射中；
- 映射目标不是 `PackageSchema`。

函数/回调类型仍返回 `CallbackAdapterUnavailable`。非 null 的字符串、数字、布尔字面量仍不能
直接作为结构边界类型；本任务没有增加 literal 例外，也没有按显示名称猜测类型。

## 真实 llm-api 验收

使用隔离 artifact store 完成 canonical official std bootstrap，并发布真实
`/Users/geek/workspace/internals-phase-05-integration/packages/llm-api`。

`responses.ResponsesMaterializationResult` 的 schema index 记录保持为公开
`PackageSchema`。两个真实 callable 的 `unsupportedBoundaryType` 均已消失：

- `responses.completedOrThrow` 仅剩独立的 caller return/throw alias 原因；
- `responses.materializeCompletedResult` 仅剩独立的 unknown call/effect、caller write/alias
  和 same-heap 原因。

因此真实四分支字符串 discriminator union 已通过名义 schema 引用处理，没有被结构内联。

## 验证

- `cargo test -p skiff-compiler-projection --no-fail-fast`：29 passed；
- `cargo test -p skiff-compiler-contract --no-fail-fast`：1 passed；
- canonical std bootstrap + 真实 llm-api publish：通过；
- `cargo check --workspace`：通过；
- `git diff --check`：通过。

未操作 shared stable instance，未 push。
