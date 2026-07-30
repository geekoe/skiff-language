# P5-F294 Applied nominal shared model result

状态：implementation complete；等待独立 `A1b-applied-nominal-model` 验收。

## Exact candidate

- implementation commit：
  `e5c36178b35bbfa55d6ce4042053ebe87e1dd257`
- 直接父任务：
  `P5-F294-applied-nominal-shared-model.md`
- owner审计：
  `P5-F293-generic-nominal-type-ref-owner-audit-result.md`

## 实现结果

- 新增唯一
  `TypeRefIr::AppliedNominal { base: NominalTypeRefBaseIr, arguments: Vec<TypeRefIr> }`；
- base只允许Local、Publication、ServiceSymbol、PackageSymbol、PackageSchema；
- arguments required、non-null、non-empty、ordered；零参数只用plain nominal；
- named-union concrete branch删除独立`type_arguments` map，只保留`nominal_type`；
- File IR local declaration admission验证kind、arity与TypeParam scope；
- external owner保留exact locator/ABI expectation；
- applied PackageSchema在本任务拥有的File IR、Actor ABI、PackageArtifact admission中fail closed；
- descriptor/signature/throw/catch/construct/DB/interface box/call args/actor ref及nested arguments traversal
  已接入；
- cross-package rebind递归保留base owner与ordered arguments。

## Identity generation

| Domain | 新代际 |
| --- | --- |
| File IR schema / format / prefix | v7 / v5 / v7 |
| PackageArtifact schema | v5 |
| Local ABI marker / prefix | v3 / v5 |
| Build marker / prefix | v4 / v6 |

opcode、legacy Package Unit、PackageSchema Type/Index、ServiceProtocol v4、ContractOperation、
Operation ABI、Publication ABI保持。Mutation tests覆盖argument type、nested/reorder、owner/argument
tamper、旧generation、non-generic新writer及human version label非identity。

## 开发证据

```text
skiff-artifact-model --lib     149/149
skiff-artifact-identity --lib   93/93
两目标crate rustfmt             PASS
git diff --check                PASS
```

全workspace fmt唯一命中当时base中未授权的compiler fixture格式差异；implementation没有修改compiler。

生产范围内旧branch map已清零；`typeArguments`只剩严格拒绝/absence测试。compiler、linked/runtime与
公共generic schema consumer均未承接，属于后续节点而非本结果PASS声明。

