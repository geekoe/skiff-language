# P5-F440C Cancellation compiler/artifact hard cut result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED`。

本 leaf 已从 compiler builtin registry 删除用户可见的 cancellation error，并在 File IR type-ref
admission checkpoint 对旧 canonical spelling fail closed。`TimeoutError` 保持注册、lowering 与 artifact
admission；runtime/linker production、scripts、公共 schema、Router、stable/live 均未修改或运行。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| task worktree 起点 | `a5fdcbd712dbcd30f6a421ee48b6b2876f970e36` | `33911300aa666a610f6ed82087682efe1153fe97` |
| implementation | `cb4b04560ae94f6e1011c2313e4286a7bec291ac` | `16eff6d4b0bb68ea5c017ba47017828a7ca0f745` |

implementation 精确修改：

- `compiler/core/src/prelude_registry.rs`
- `compiler/source/src/type_resolution_model.rs`
- `compiler/lowering/src/type_lowering.rs`
- `compiler/tests/builtin_canonical_spelling.rs`
- `artifact-model/src/file_ir.rs`
- `artifact-model/src/file_ir/legacy_builtin_tests.rs`

除此之外只新增本文 result。

## 2. 实现结果

### 2.1 Compiler public surface hard cut

- `COMPILER_BUILTIN_TYPES` 不再含 name `CancelError`、symbol `std.error.CancelError` 或对应 `Error`
  kind member。
- 短名与 qualified spelling 均不能解析；constructor、普通 type、throw payload、catch type、
  `Exception<E>` rethrow envelope 和 catch union leaf 都有直接负例。
- 删除 registry member 后，通用的 compiler-owned namespace guard 阻止未知 `std.*` / `config.*`
  名字被误降级为 package/service symbol。该 guard 不包含 cancellation 名称特判或隐藏 alias。

### 2.2 Artifact admission hard cut

- `validate_file_ir_type_refs` 在访问任意 `TypeRefIr::Builtin` node 时，精确拒绝 retired canonical
  spelling `CancelError`。
- validator 的既有递归 visitor 覆盖普通 descriptor、Throw payload、Catch catch type，以及
  record/nullable/union 多层嵌套。
- tombstone 只报错，不 lower 为 `unknown`、其它 builtin 或 native identity。
- 只读核对 `runtime/linker/src/linker/file_conversion.rs:15`：linker conversion 的第一项动作仍是调用
  `validate_file_ir_type_refs(unit)`，因此旧 spelling 在形成 `LinkedTypeRef::Native` 前失败；本 leaf
  没有修改 linker。

### 2.3 TimeoutError 不变量

- registry 仍精确拥有 `TimeoutError` / `std.error.TimeoutError`、arity `0`、kind `Error`。
- 既有 qualified lowering 正例仍观察 canonical `TimeoutError` File IR builtin。
- artifact 正例把 `TimeoutError` 放入与 legacy probe 相同的普通、Throw、Catch、nullable/union
  carrier，validation 继续通过。

## 3. 测试先行与验证

### 3.1 真实 red evidence

在 production 修改前执行新增/改写后的测试：

| Suite | Red 结果 |
| --- | --- |
| `cargo test -p skiff-compiler --test builtin_canonical_spelling -- --nocapture` | `3 passed, 5 failed`；type/throw/catch/rethrow/union 仍可发射，constructor 已由既有规则拒绝 |
| `cargo test -p skiff-artifact-model file_ir::legacy_builtin_tests:: -- --nocapture` | `1 passed, 4 failed`；四个 legacy carrier 均被 validator 接受，Timeout 正例通过 |

这两组失败均直接来自待删除行为，不是 compile error、零测试 selector 或 skip。

### 3.2 精确 test listing

`cargo test -p skiff-compiler --test builtin_canonical_spelling -- --list` 列出 9 tests、0 benchmarks：

```text
cancel_error_short_and_qualified_catch_types_are_rejected
cancel_error_short_and_qualified_constructors_are_rejected
cancel_error_short_and_qualified_rethrow_envelopes_are_rejected
cancel_error_short_and_qualified_throw_payloads_are_rejected
cancel_error_short_and_qualified_type_spellings_are_rejected
cancel_error_short_and_qualified_union_leaves_are_rejected
compiler_builtin_registry_retires_cancel_error_and_keeps_timeout_error
declared_source_aliases_emit_only_canonical_file_ir_builtin_names
undeclared_builtin_spellings_are_not_implicit_source_aliases
```

`cargo test -p skiff-artifact-model file_ir::legacy_builtin_tests:: -- --list` 列出 5 tests、0 benchmarks：

```text
file_ir::legacy_builtin_tests::file_ir_keeps_timeout_error_admitted_in_the_same_carriers
file_ir::legacy_builtin_tests::file_ir_rejects_retired_cancel_error_in_catch_type
file_ir::legacy_builtin_tests::file_ir_rejects_retired_cancel_error_in_nested_union
file_ir::legacy_builtin_tests::file_ir_rejects_retired_cancel_error_in_ordinary_type_ref
file_ir::legacy_builtin_tests::file_ir_rejects_retired_cancel_error_in_throw_payload
```

### 3.3 Green evidence

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler --test builtin_canonical_spelling` | PASS：9 passed、0 failed |
| `cargo test -p skiff-artifact-model file_ir::legacy_builtin_tests::` | PASS：5 passed、0 failed、168 filtered out |
| `cargo check -p skiff-artifact-model -p skiff-compiler-core -p skiff-compiler-source -p skiff-compiler-lowering -p skiff-compiler` | PASS；仅仓库既有 unused/dead-code warnings |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

聚焦测试合计 14 passed、0 failed。

## 4. Reverse search

对 `compiler/**`、`artifact-model/**` 搜索 `CancelError|std\.error\.CancelError`，最终只剩 5 处、3 类：

1. `artifact-model/src/file_ir.rs` 的 1 个 admission tombstone；
2. `artifact-model/src/file_ir/legacy_builtin_tests.rs` 的 2 个明确 negative rejection test 文字；
3. `compiler/tests/builtin_canonical_spelling.rs` 的 2 个明确 negative spelling matrix。

排除 tests 与 tombstone 后，对 registration/emission 形态
`name: "CancelError"`、`symbol: "std.error.CancelError"`、`TypeRefIr::builtin("CancelError")` 和
内联 `TypeRefIr::Builtin ... CancelError` 的搜索结果为 0。compiler production 不再注册、解析或发射
该 spelling。

`compiler/core/src/prelude_registry.rs` 的 `TimeoutError` name/symbol 两行仍在；linker production
对 `validate_file_ir_type_refs(unit)` 的最早调用仍在。

## 5. Scope 与禁令

- 没有修改 runtime/linker production、runtime、router、scripts、std、test-runner、公共 artifact schema
  generation 或其它 task/result。
- 没有运行完整 verify、Router、live、instance、stable 或昂贵 combined gate。
- 没有 merge、rebase、push 或注册 stable watch。
- implementation 与 result 分开提交；result commit/tree 由交付消息记录。
