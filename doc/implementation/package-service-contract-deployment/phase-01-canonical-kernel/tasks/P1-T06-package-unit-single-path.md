# P1-T06：PackageUnit 单一 Projection Path

## 目标

删除production与package-test两套PackageUnit/ABI/export/identity组装路径，让测试设施消费同一production
projection，而不是维护一套看似相同的builder。

## 依赖与 worktree

- 依赖P1-T02A、P1-T03、P1-T04、P1-T05。
- 从包含四项前置提交的phase checkpoint建task worktree；必须在T02A与T05之后执行。
- 建议branch：`codex/package-service-p1-t06-package-unit-single-path`。

## 完成态

1. `compiler/projection/src/package_unit_artifacts/**`是唯一PackageUnit projection owner；按source facts、
   export index、ABI surface、implementation links和artifact assembly拆成职责清晰模块。
2. 删除`compiler/projection/src/typed_artifacts/package_unit.rs::build_package_unit`或使该文件完全消失；
   package-test emission调用production projection API并只叠加test assembly facts。
3. public-instance receiver/conformance/interface operation只组装一次；测试路径不重算publication/package
   ABI或identity。
4. projection/emission/lowering中的identity adapter只能是无逻辑re-export或直接调用canonical owner；删除
   panic unwrap、prefix拼接和重复assign helper。
5. existing production package artifact与package-test production package对同一input得到相同typed
   PackageUnit和identity。
6. 直接触碰的`package_unit_artifacts.rs`被拆分，不能继续增长单个千行文件。
7. 此任务是T05 package identity API进入compiler production/package-test路径的唯一adoption owner；
   T03已经完成现有路径的typed effect wire迁移，本任务只消费该shape并在删除双builder时保持parity。

## 写入范围

- compiler projection package unit/package exports/typed artifacts modules。
- compiler emission package/package-test artifact modules。
- 直接test support和compiler/package-test tests。

不要改变T03/T05已冻结的effect shape或identity preimage，也不要重新迁移effect wire、改变service
projection或runtime。

## 验证

```bash
cargo fmt --all -- --check
cargo test -p skiff-compiler-projection -p skiff-compiler-emission
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
git diff --check
```

增加compiler production/package-test producer parity测试，覆盖functions、public instances、nominal types、
dependencies、config/effects和resources。runtime package-test/test-runner consumer由T07迁移并验收。

## 自验收与回报

反向搜索`fn build_package_unit(`、`package_export_index_from_file_units`重复实现、package-test手工ABI/identity
assign和typed_artifacts facade。提交自验收矩阵、production owner证据和commit。
