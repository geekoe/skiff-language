# P5-F160：Package Schema Content-addressed Store Result

状态：Completed

## 直接父任务

- `P5-F160-package-schema-store.md`

## 交付

- 新增两个独立canonical record path：
  - `PackageSchemaIndexRecordPath`
  - `PackageSchemaTypeRecordPath`
- 两类路径都由owner package id与对应content identity组成，不引入version、release pointer或新的发布对象。
- `CanonicalArtifactStore`新增Package schema index/type record严格读写：
  - 写入前重新计算并校验canonical identity；
  - 同路径同bytes幂等；
  - 同路径不同bytes返回immutable conflict；
  - 读取时先校验raw JSON中的owner与path identity，再做strict typed decode、canonical identity重算和
    canonical JSON bytes校验；
  - type record identity重算同时覆盖stable schema key与canonical descriptor。
- 新增`resolve_package_artifact_schema`完整解析入口：
  - 先验证PackageArtifact identity；
  - index owner必须与PackageArtifact一致；
  - index entry type id集合必须与artifact record ref集合精确相等，重复index type id也拒绝；
  - 每个index stable key、type id、artifact ref和已读取record必须逐项一致；
  - 最终对resolved record集合执行canonical identity、owner/key、传递闭包与无环校验；
  - 只返回已验证的index和逐类型record，不执行compiler、deployment projection或runtime语义。
- 两个不同version的PackageArtifact可解析同一content-addressed type record；两次写入落到同一路径且
  不产生重复payload。

## 验证

通过：

```text
cargo test -p skiff-artifact-identity --lib
72 passed; 0 failed

独立storage-only cargo check（临时检查crate不依赖compiler-contract）
passed

git diff --check
passed
```

新增store聚焦测试覆盖：

- index/type record round-trip与幂等写入；
- immutable payload冲突；
- 缺index、缺type record；
- type record错path、错owner、错stable key、错descriptor hash；
- index entry与artifact ref、resolved record不一致；
- artifact多余或缺失record ref；
- 两个PackageArtifact复用同一个type record path与payload。

未能在本节点执行：

```text
cargo test -p skiff-deployment --lib
```

该命令在编译`skiff-compiler-contract`时先命中P5-F159 result已记录的14处旧
`ContractTypeId`/`boundarySchema`迁移断面，尚未编译到deployment或本任务测试。按任务边界未恢复旧类型、
未增加伪兼容，也未修改compiler projection。compiler恢复编译后需由后续独立gate补跑上述store聚焦测试。
