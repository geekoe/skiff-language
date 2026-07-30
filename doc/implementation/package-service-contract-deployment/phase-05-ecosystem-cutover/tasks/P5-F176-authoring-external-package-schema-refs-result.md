# P5-F176：Authoring External Package Schema Refs Result

状态：Completed

## 直接父任务

- `P5-F176-authoring-external-package-schema-refs.md`

## 交付

- Package schema projection只为当前Package中具有可编码boundary descriptor的公开named type生成
  record；descriptor闭包按当前Builtin模型递归校验，`Stream`、function、interface value、DB对象、
  local index等无boundary descriptor类型不生成record。
- 显式actor declaration没有普通type descriptor，因此不会进入Package schema definitions，也不会
  被伪造成当前Package-owned record。
- descriptor中的外部`PackageSymbolRef`必须精确命中driver已验证的
  `ResolvedPackageSchema`公开type entry，并直接生成保留外部package owner、stable key和
  `PackageSchemaTypeId`的`PackageSchema`引用；不复制外部record、不改owner。
- Package dependency alias必须唯一命中；只带PackageId的引用若命中多个精确binding会fail closed，
  不再静默选择第一个版本。
- 最终schema闭包使用当前Package records和全部validated dependency records联合验证，拒绝type
  identity碰撞、缺record、owner/key/type id不一致；descriptor在计算本地type id前执行canonical
  contract shape normalization。
- P5-F149手写JSON fixture补齐当前必需的`packageSchemaIndex`与
  `packageSchemaTypeRecords` wire字段，没有使用serde default隐藏缺失字段。

## 验证

通过：

```text
cargo test -p skiff-compiler-projection package_artifact::schema -- --nocapture
# 5 passed

cargo test -p skiff-compiler --lib p5_f149 -- --nocapture
# 2 passed

cargo check -p skiff-compiler-projection
cargo check --workspace
git diff --check
```

覆盖跨Package公开type引用成功、owner不被复制、PackageId多精确binding歧义拒绝，以及stream和
actor handle零record。

完整`cargo test -p skiff-compiler --lib`中，F149两项已修复；其余authoring测试仍在初始化
platform sources时被真实`std/actor.skiff`的旧`native type ActorRef`语法阻断。该文件属于P5-F179
迁移范围，本任务按约束未修改std，待F179合入integration后复跑完整compiler lib测试。
