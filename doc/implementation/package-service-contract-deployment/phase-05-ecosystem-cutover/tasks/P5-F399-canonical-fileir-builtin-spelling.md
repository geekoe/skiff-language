# P5-F399 Canonical FileIR builtin spelling

状态：Ready。

## 直接父节点

- `P5-F398-builtin-canonical-spelling-audit-result.md`

父节点已确定producer唯一owner：semantic resolution正确，FileIR lowering错误地保留raw source alias。
本节点统一compiler producer；PackageSchema与Runtime linker保持strict exact。

## Worktree

- `/Users/geek/workspace/skiff-p5-f399-canonical-builtin-spelling`
- branch `codex/p5-f399-canonical-builtin-spelling`
- base：包含本任务的Skiff phase-05 integration。

## Production要求

1. `compiler/core/src/prelude_registry.rs`
   - 建立唯一、可枚举的source spelling→canonical FileIR builtin registry；
   - primitive `boolean|bool -> bool`；
   - 18个compiler builtin bare/symbol forms归一到canonical name；
   - arity/kind保持、alias无碰撞、unknown返回None。
2. `compiler/source/src/type_resolution_model.rs`
   - 只消费core registry；
   - 删除重复builtin/qualified表和未声明隐式alias。
3. `compiler/lowering/src/type_lowering.rs`
   - 所有AST/text→`TypeRefIr::Builtin`入口递归canonical；
   - 覆盖bare/qualified、generic、nullable、union、record、function nested refs；
   - 不再“recognized后原样返回input”。
4. emitted FileIR/package artifact递归不得出现任何registry alias spelling。

## 严格边界

不修改：

- `std/db.skiff`
- PackageSchema/contract normalization
- `artifact-model::TypeRefIr`
- Runtime linker exact comparison
- reader compatibility/tolerant alias
- stable/live。

若必须重写无关lowering pipeline或artifact schema，返回`TASK_SCOPE_EXPANDED`。

## 测试与fresh验证

覆盖父节点§Implementation tests全部矩阵，至少运行：

```bash
cargo test -p skiff-compiler-core prelude_registry
cargo test -p skiff-compiler-source prelude_registry
cargo test -p skiff-compiler --test builtin_canonical_spelling
cargo test -p skiff-compiler --test prelude_std_schema prelude_builtin_schema_is_typed_in_file_ir
cargo test -p skiff-compiler --test prelude_std_schema builtin_types_reach_the_package_boundary_projection
cargo test -p skiff-runtime-linker service_error_index
cargo test -p skiff-test-runner --test canonical_std_seed_bootstrap -- --test-threads=1
git diff --check
```

fresh std验收：

- `ConflictError.retryable`的FileIR/linked/Local ABI/PackageSchema均为`bool`；
- artifact递归`boolean`及其它alias为零；
- PackageSchema type/index identity保持bit-identical；
- FileIR、Local ABI、build按预期变化；
- strict linker仍拒绝人工noncanonical pair；
- isolated std activation越过ServiceErrorTypeIndex gate。

写`P5-F399-canonical-fileir-builtin-spelling-result.md`，production/tests/result本地commit，worktree
clean；不merge/rebase/push，不派子Agent。
