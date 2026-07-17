# P1-T05：Package Identity Projection

## 目标

用显式字段矩阵重建package local ABI/build identity，修复semantic facts遗漏和storage/provenance误入，
并为所有declared package identity提供assign/validate同源API。

## 依赖与 worktree

- 依赖P1-T01、P1-T02、P1-T03。
- 从包含三项前置提交的phase checkpoint建task worktree。
- 建议branch：`codex/package-service-p1-t05-package-identity`。

## 完成态

1. `artifact-identity`定义独立、typed的`PackageLocalAbiIdentityProjection`与
   `PackageBuildIdentityProjection`，字段与phase plan §4完全一致，不直接serde整个PackageUnit。
2. local ABI identity包含package id/version coordinate、public surface和`AbiIdentityFacts`；不再简单等于
   legacy publication ABI hash。相同内容使用不同coordinate必须得到不同local ABI/build identity。
3. build identity包含local ABI identity、File IR identities、implementation links、package dependency
   expectations、config/resource/runtime requirements、recoverable metadata和typed effect facts。
4. build preimage对FileIrRef只使用`fileIrIdentity`与module owner；ref-level `artifactPath`、重复的
   `sourceAstHash`、diagnostic wording和map insertion order不影响结果。File IR identity内部已选择的
   source-map/debug content仍由File IR owner决定，不在本任务二次过滤。
5. 提供`assign_package_unit_identities`与`validate_package_unit_identities`的同源实现；nested publication/
   operation identity也必须先验证，不能把不可信declared string吞入外层hash。
6. identity语义变化直接更新schema marker/prefix和golden；不保留v1兼容计算或fallback。
7. mutation-matrix test逐字段证明include/exclude行为，测试数据不是只覆盖happy path。

## 写入范围

- `artifact-identity` package identity模块、errors、tests/goldens。
- artifact-model中仅为typed projection所需的accessor/leaf调整。
- 仅为identity API本身所需的artifact-identity fixtures/goldens。

不要修改compiler projection/emission adoption callsite或PackageUnit builder；T06会在T05 API冻结后统一采用。
也不要修改runtime/router pointer、ServiceUnit/serviceAssembly identity或effect analyzer。

## 验证

```bash
cargo fmt --all -- --check
cargo test -p skiff-artifact-model -p skiff-artifact-identity
node scripts/check-artifact-identity-single-source.mjs
git diff --check
```

必须单独断言：nominal fact改变ABI/build；recoverable/effect改变build但不改变ABI；artifact-ref path/
ref-level source hash/diagnostic detail不改变；相同内容不同package id/version coordinate改变ABI/build；篡改
nested identity、ABI或build identity均被validate拒绝。

## 自验收与回报

提交字段矩阵对应的代码行证据和mutation tests；反向搜索直接`serde_json::to_value(PackageUnit)`作为identity
preimage、`package_abi_hash == publication_abi_hash`假设和旧prefix。提交自验收矩阵与commit。
