# P1-T02：Nominal / Callable Identity 单一 Owner

## 目标

把nominal type/interface/callable/method与operation surface identity算法全部归`artifact-identity`，
让artifact-model只保存typed input/DTO，producer和dependency consumer共享同一validator。

## 依赖与 worktree

- 依赖 P1-T01；从已合入T01的phase checkpoint建task worktree。
- 建议branch：`codex/package-service-p1-t02-semantic-identity`。

## 完成态

1. `artifact-model/src/abi_identity.rs`只保留serializable typed inputs/newtypes和不变量，不再生成framed
   bytes、hash、hex或stable string。
2. `type_ref_abi_key`、interface instantiation和method/callable identity不再通过
   `serde_json::to_string + format!`散落生成；canonical derivation由`artifact-identity` API拥有。
3. `compiler/projection/src/contract/abi_projection.rs`不再自己hex key bytes；source/projection/ABI builder
   只调用canonical API。
4. operation/package surface有一个纯leaf validator，至少校验duplicate operation id、source-call
   index target、public-instance method target、schema closure key及declared identity。
5. 增加`validate_publication_abi_identity`；compiler producer在assign后可验证，
   `compiler/input/src/service_dependencies.rs`在信任dependency前必须recompute/validate，而不是只检查非空。
6. 删除无调用的`compiler/driver/shared/operation_abi_identity.rs`及不必要的多层framing wrapper。
7. `PublicationAbiUnit`本阶段仍是legacy aggregate；不得把它抽象成四个目标对象的共同父类型。

## 写入范围

- `artifact-model/src/abi_identity.rs`、`publication_abi.rs`及直接tests/reexports。
- `artifact-identity` semantic/operation模块。
- `compiler/publication-abi`、contract ABI projection、service dependency input和直接thin adapters。
- 删除确认无调用的driver helper。

不要修改package build/local ABI inclusion matrix、effect model、PackageUnit builder、runtime/router。

## 验证

```bash
cargo fmt --all -- --check
cargo test -p skiff-artifact-model -p skiff-artifact-identity
cargo test -p skiff-compiler-publication-abi -p skiff-compiler-input -p skiff-compiler-projection
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-compiler-boundaries.mjs
git diff --check
```

测试必须覆盖同anchor同args稳定、不同anchor不同、public path不影响nominal id、descriptor变化不改变
nominal id但改变contract revision、method generic args有序、篡改declared identity fail closed，以及
duplicate/dangling operation refs。

## 自验收与回报

反向搜索`abi_id_key_hex`、`type_ref_abi_key`算法实现、operation identity string format和旧driver helper；
说明每个剩余命中为何只是DTO/display/test。提交自验收矩阵和commit。
