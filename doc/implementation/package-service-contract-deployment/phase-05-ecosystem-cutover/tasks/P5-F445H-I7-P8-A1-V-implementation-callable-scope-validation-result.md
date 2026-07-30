# P5-F445H I7 P8 A1-V Implementation callable scope validation result

状态：

```text
PASS
A1_V_COMPLETE = YES
A1_RESUME_UNBLOCKED = YES
AGINE_170_RESUME_UNBLOCKED = NO
SCHEMA_MODEL_CANONICAL_PROJECTION = NO_OP
```

## 1. Exact input and implementation

- baseline commit：
  `15b0f3934b96e72262ff478eee9a50b46ebf0b41`
- baseline tree：
  `aec0b78d7191dd99049cbf5885bc8cac6e15a34c`
- implementation commit：
  `85c40cfe6cda53dfe328d7f713e236f83aed00fa`
- implementation tree：
  `41bf089f7e6f0f6f64f1a005404b2d60683237e3`

production写集只有：

```text
artifact-identity/src/package_artifact/validation.rs
```

直接测试写集只有：

```text
artifact-identity/src/package_artifact.rs
```

没有修改artifact model/schema、identity projection、compiler、source/lowering、linker、runtime或
test-runner。

validator现在：

1. 强制每个implementation callable使用
   `pkg-callable:<packageId>:top-level:<sourcePath>`；
2. 以每个精确callable id分别解析public与implementation surface，并要求恰好一个owner；
3. 对同一executable上的每个可参与link的callable signature分别验证File IR type-parameter scope；
4. 只允许implementation-only impl callable以`ImplMethod`指向精确
   `implementationLinks.implMethods` coordinate；
5. 以public与implementation `ImplMethod` target并集闭合method target coverage；
6. 保留public function、public-instance、普通implementation-only `InternalFunction`、
   callableLinks、semantic facts、boundary projection与canonical identity原有隔离。

## 2. Stable RED and GREEN

production修改前，新增A1同形fixture稳定得到结构化
`InvalidPackageArtifact`，并精确包含：

```text
pkg-callable:example.identity:top-level:api.Worker.run
without a Local ABI signature
```

production修改后，`implementation_only_impl_callable_scope` selector为：

```text
6 passed; 0 failed
```

覆盖：

- public method id与implementation-only impl id共享同一File IR executable；
- 纯implementation-only private impl method；
- duplicate、missing、non-callable和wrong owner；
- wrong target；
- `InternalFunction`、`PublicFunction`、`ReceiverMethod`三种wrong kind；
- 多个exact callable scope逐一验证和不兼容scope；
- implementation-only id不进入`publicSymbols`或`boundaryProjections`；
- id、signature、target与kind变异拒绝或改变build identity。

## 3. Validation evidence

| command | result |
| --- | --- |
| `cargo test --locked -p skiff-artifact-identity implementation_only_impl_callable_scope -- --nocapture` | PASS，6/6 |
| `cargo test --locked -p skiff-artifact-identity implementation_link_type_parameters_use_the_matching_public_callable_scope -- --nocapture` | PASS，1/1 |
| `cargo test --locked -p skiff-artifact-identity implementation_symbol_callable_type_parameter_scope_is_validated -- --nocapture` | PASS，1/1 |
| `cargo test --locked -p skiff-artifact-identity package_artifact -- --nocapture` | PASS，26/26 |
| `cargo test --locked -p skiff-artifact-identity -- --nocapture` | PASS，142 unit + 8 CLI；1 ignored fixture regenerator |
| `cargo check --locked -p skiff-artifact-identity` | PASS |
| `rustfmt --edition 2021 --check artifact-identity/src/package_artifact.rs artifact-identity/src/package_artifact/validation.rs` | PASS |
| `git diff --check` | PASS |

`cargo fmt --all -- --check`已执行，唯一失败是baseline已有且不属于冻结写集的
`compiler/tests/package_imports.rs`三处格式差异。该文件在精确baseline已经是同样内容；A1-V没有修改或提交
它。A1-V两个实际代码文件的直接rustfmt check通过。

## 4. Handoff

A1-V已闭合artifact identity validator缺口，可以恢复A1原compiler owner。A1-V不恢复Agine 170，不运行
A1 compiler GREEN、canonical source fixture、Agine、J gate或stream lane。
