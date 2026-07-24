# P5-F163：Package Schema Projection Recovery Result

状态：Completed

## 直接父任务

- `P5-F163-package-schema-projection-recovery.md`

## 交付

- compiler PackageArtifact projection现在从typed `api.yml` public type graph生成：
  - 以canonical public path为`stableSchemaKey`的`PackageSchemaTypeRecord`；
  - child-first计算的`PackageSchemaTypeId`；
  - 只含`PublicNameable` entry的`PackageSchemaIndex`；
  - PackageArtifact中的exact index/type record refs。
- 本Package named child必须能反查到`api.yml`公开path；找不到立即fail closed。v1递归或互递归schema在
  visiting stack处拒绝。
- dependency与implicit std named type只通过F162的`ResolvedPackageSchema::public_type`取得
  `packageId + stableSchemaKey + PackageSchemaTypeId`；projection不访问filesystem/store，也不再用HTTP
  结构特判生成身份。
- callable boundary删除旧`PackageTypeRef::Contract`与`ContractTypeRef::PackagePublic`路径，直接保留
  Package-owned schema ref。
- compiler/contract删除service-owned type id、`boundarySchema`复制及canonicalization：
  - operation中的Package ref原样保留；
  - 从operation roots沿已验证record descriptor计算精确传递闭包；
  - 按Package owner生成排序、去重的`PackageTypeRequirement`；
  - ServiceContract不再拥有schema descriptor。
- projection/emission保留本次生成的schema实体；authoring publication在顶层PackageArtifact之前调用F160
  canonical store writer写入type records与index。

## 验证

通过：

```text
cargo test -p skiff-compiler-projection -p skiff-compiler-contract
20 passed; 0 failed

cargo test -p skiff-artifact-model -p skiff-artifact-identity
artifact-model 109 passed
artifact-identity 72 passed; CLI 8 passed

cargo check -p skiff-compiler-emission --lib
passed

git diff --check
passed
```

聚焦证据覆盖Package type identity不随service id或version label变化；artifact identity既有测试继续覆盖：

- 未引用index entry不改变protocol；
- 引用descriptor变化改变type id/protocol；
- Package version/build不进入type id；
- recursive record与不完整闭包fail closed。

## 下一层编译断面

完整`skiff-compiler`仍按F159硬切预期停在尚未迁移的consumer/deployment范围：

- `compiler/input/contract_dependencies`仍读取已删除的`ContractTypeId`和`boundarySchema`；
- deployment operation projection仍导入已删除的service-owned canonicalizer；
- deployment WebSocket ingress尚未把resolved Package schema records传给新的严格解析入口。

本节点没有恢复兼容wire或越界修复这些consumer。下一任务应迁移dependency consumer materialization和
deployment/runtime schema record输入后，再补跑真实source compiler pipeline及official std bootstrap。
