# P5-F175：Workspace Package Schema Test Fixtures Result

状态：Completed

## 直接父任务

- `P5-F175-workspace-package-schema-test-fixtures.md`

## 交付

- 迁移compiler emission、compiled与workspace integration tests中的旧service-owned schema fixture；
  有命名边界类型的fixture现在使用真实Package owner、canonical content id、schema index、
  requirements与resolved record map。
- 迁移runtime host assembly admission、router session、linked-program、linker及package-test的共享
  fixture；无命名边界类型的Package使用按真实package identity计算的合法空schema。
- host typed error、stream与callback fixture现在由Package schema record驱动resolver和deployment
  projection；错误tuple变体会重算canonical record id并同步更新引用，避免构造内部不一致的fixture。
- linked-program新增仅用于测试的artifact-identity依赖，以复用生产canonical empty schema计算。
- 未修改production校验或运行时行为。

## 验证

通过：

```text
cargo test --workspace --no-run
cargo check --workspace
git diff --check
```

针对性执行通过：

```text
cargo test -p skiff-runtime-linked-program
# 18 passed

cargo test -p skiff-compiler-emission
# 11 passed

cargo test -p skiff-runtime-linker assembly
# 12 passed

cargo test -p skiff-runtime-host assembly_admission
# schema迁移相关测试25项直接通过；唯一错误tuple fixture修正后，失败用例单独复测通过
```

## 已知既有运行失败

`cargo test -p skiff-compiler-compiled --test public_instance_signature_handoff`已成功编译，但运行时在
`compiler/source/src/prelude_registry/initialization.rs`因测试未提供`CompilerPlatformSources`而触发
“prelude registry is not initialized”。该失败发生在进入本任务迁移的Package schema断言之前，
不属于fixture schema字段缺失或旧service-owned schema构造问题；workspace `--no-run`门禁已通过。
