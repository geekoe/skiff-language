# P5-F445H I7 P8 A1-V-R1 Public function internal alias regression result

状态：

```text
PASS
A1_V_R1_COMPLETE = YES
A1_RESUME_UNBLOCKED = YES
S3_RESUME_UNBLOCKED = YES
AGINE_170_RESUME_UNBLOCKED = NO
SCHEMA_COMPILER_RUNTIME = NO_OP
```

## 1. Input and cause

- baseline commit：
  `0f03171608790013ea8974018efb51369a76d15b`
- baseline tree：
  `7a8318655c668a78fe4de570096dbf29cc279177`
- implementation commit：
  `227cb96f337a0051f279bee9e64a2af0f7068758`
- implementation tree：
  `3940ea0c6973f5ef5dd5a6dff4cb5a391d53f783`

A1-V把所有位于`implementationLinks.functions` coordinate上的implementation
`InternalFunction`都判为非法，误伤canonical std的合法形状：

```text
public callable
  kind = PublicFunction
implementation top-level callable
  kind = InternalFunction
same File IR file/executable coordinate
```

稳定RED精确复现：

```text
implementation callable
pkg-callable:example.identity:top-level:api.run
uses InternalFunction for an exported implementation target
```

## 2. Repair

只修改：

```text
artifact-identity/src/package_artifact/validation.rs
artifact-identity/src/package_artifact.rs
```

`validate_public_callable_link_kinds`现在允许implementation `InternalFunction`与同coordinate的
精确public `PublicFunction`共存。以下边界保持fail closed：

- `InternalFunction`指向`implementationLinks.implMethods`；
- exported function coordinate缺少精确public `PublicFunction` owner；
- implementation callable错误id、surface、source owner或signature；
- implementation-only `ImplMethod`错误kind/target；
- implementation-only id污染`publicSymbols`或`boundaryProjections`。

没有修改schema、artifact model、identity projection、compiler projection/source/lowering、
runtime或std source。

## 3. Evidence

| command | result |
| --- | --- |
| `cargo test --locked -p skiff-artifact-identity implementation_internal_function_alias -- --nocapture` | PASS，2/2 |
| `cargo test --locked -p skiff-artifact-identity implementation_only_impl_callable_scope -- --nocapture` | PASS，6/6 |
| `cargo test --locked -p skiff-artifact-identity package_artifact -- --nocapture` | PASS，28/28 |
| `cargo test --locked -p skiff-artifact-identity -- --nocapture` | PASS，144 unit + 8 CLI；1 ignored fixture regenerator |
| `cargo test --locked -p skiff-compiler official_std_authoring_and_record_writer_are_fixed_and_deterministic -- --nocapture` | PASS，1/1；canonical std exact build id不变 |
| `cargo check --locked -p skiff-artifact-identity` | PASS |
| `rustfmt --edition 2021 --check artifact-identity/src/package_artifact.rs artifact-identity/src/package_artifact/validation.rs` | PASS |
| `git diff --check` | PASS |

`cargo fmt --all -- --check`仍只报告baseline已有的
`compiler/tests/package_imports.rs`三处格式差异；该文件不在本任务写集，R1未修改或提交它。

## 4. Handoff

R1已解除canonical std authoring blocker，可以恢复A1与S3。Agine 170仍等待这些上游节点完成，不由R1启动。
