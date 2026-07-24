# P5-F173：Runtime Eval Package Schema Fixtures

状态：Ready

## 直接父任务

- `P5-F171-runtime-eval-package-schema-materialization-result.md`

## 当前断点

eval生产代码已越过Package schema cutover，但ordinary、projection和spawn的通用测试fixture仍构造
缺少`package_schema_index`/`package_schema_type_records`的旧`PackageArtifact`，使eval测试二进制
无法编译并遮挡F171/F172真实测试。

## 范围

只修改报错的eval测试fixture：
`assembly_execution/ordinary/tests.rs`、`assembly_execution/projection.rs`、
`spawn_ops/canonical_tests.rs`，以及必要的共享测试helper；不得修改生产逻辑。

## 必须实现

- fixture使用当前PackageArtifact模型，显式提供与fixture public API一致的schema index/type records。
- 无命名public类型的fixture显式使用合法空schema；需要命名类型时必须使用真实canonical records。
- 不得用默认空值掩盖本应存在的contract requirement或跳过admission校验。

## 验证

- `cargo test -p skiff-runtime-eval --no-run`越过这些fixture；
- 能运行的eval聚焦测试；
- `git diff --check`；
- 独立提交并写result。
