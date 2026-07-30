# P5-F297 Applied nominal linked/type-plan consumer result

状态：implementation complete；完整crate tests仍由下游旧runtime consumers部分遮挡。

## Exact candidate

- implementation commit：
  `e796a8930adf610a6b779c39fa885644dd6be1e3`
- 直接父任务：
  `P5-F297-applied-nominal-linked-consumer.md`

## 结果

- 新增canonical
  `LinkedTypeRef::AppliedNominal { base, arguments }`与closed linked base；
- linker递归link arguments，只把base解析为exact Address，不把wrapper降成bare Address；
- package owner与ABI expectation继续走严格owner解析；
- linked descriptor精确保留Record、Representation、Union、Alias、Interface及三类named-union branch；
- type plan递归substitute nested arguments，保留representation/union owner context并展开transparent alias；
- empty/wrong arity、illegal kind、unbound parameter、unresolved base与PackageSchema均fail closed；
- File IR到linked image的TypeRef surface已接入AppliedNominal traversal。

反向搜索中没有旧union `variants`、branch `type_arguments`或source/display/shape参数恢复路径。

## 证据

```text
skiff-runtime-linked-program              25/25
isolated linked-type-plan owner            11/11
isolated linker source compile              PASS
linker file-conversion focused             10/10
git diff --check                            PASS
```

标准crate入口当时被以下下游consumer遮挡：

- runtime loader仍引用closed `BoundaryErrorContract` / `contract.errors`；
- runtime boundary仍使用旧`TypeIdentity`与旧type-plan identity fields；
- throw/call site、required catch和linked-type-plan旧error identity还需要runtime carrier/catch节点迁移。

这些遮挡没有在本节点越界修补。runtime value carrier、catch identity、service error channel与wire均未实现；
本结果只解除后续runtime carrier/catch节点，不能单独宣称runtime验收通过。
