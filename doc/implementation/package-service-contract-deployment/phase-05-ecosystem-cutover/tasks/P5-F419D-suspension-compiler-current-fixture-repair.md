# P5-F419D Suspension compiler current fixture repair

状态：Ready。

## 直接父节点与既定语义

- combined失败证据：
  `P5-F419A-suspension-consumer-combined-gate-result.md`；
- Package nominal FileIR终态：
  `P5-F250-package-nominal-object-lowering-target-result.md`。

F250已经明确：exact `PackageTypeRef::PackageSchema` 在 FileIR executable signature中必须投影为携带
canonical owner与stable schema key的 `TypeRefIr::PackageSymbol`，不能退化为 builtin `unknown`。本节点
只把两个过时 integration fixture迁移到该既定终态，并修掉旧 service contract effect-bit mutation；
不再把此事作为待决设计。

## 精确起点与独占范围

- integrated start：
  `b7f7530d4b28b5c84e849a0ea2358c02ed435193`；
- combined production：
  `2b9d29eea9a65ab323240f1e6c34b3e3b29c7403`；
- F250 implementation：
  `fc34744187ca7a89a29b839e16e4c5716e0e0235`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明四个commit均为HEAD ancestor（若F250已被包含但不在当前可见短历史，仍以
`merge-base --is-ancestor`为准）。

唯一允许写入：

```text
compiler/tests/service_conformance.rs
compiler/tests/file_ir_execution_type_representation.rs
本任务 result
```

禁止修改production、validator、runtime、deployment、tooling或设计；不得派子 Agent、merge/rebase/push、
stable/live。

## 精确修复

1. `protocol_identity_tracks_semantics_but_not_diagnostic_text`：
   - 删除对旧 `BoundaryOperationContract.may_suspend` 的test-only访问；
   - 用仍属于 code-free protocol shape 的最小合法mutation证明protocol identity改变；
   - diagnostic text变化仍不得改变identity。
2. FileIR fixture：
   - consumer package继续使用 `example.com/file-ir-execution-types`；
   - contract schema seed使用独立package id；
   - consumer `package.yml` 声明该schema owner的canonical direct package dependency；
   - ServiceContract `PackageTypeRequirement`、resolved schema与manifest dependency逐跳使用同一owner；
   - executable的contract parameter/return与nested container leaf断言改为F250规定的exact
     `TypeRefIr::PackageSymbol`，包括canonical owner/alias与stable schema key；
   - impl receiver仍是本地 `LocalType`，与contract parameter的PackageSymbol角色明确区分；
   - FileIR不得携带 `PackageSchemaTypeId`或service symbol，不能把PackageSchema重新改成unknown。
3. 测试名称、helper名称和失败信息要反映“保留package nominal execution identity”，不能继续声称
   “opaque unknown representation”。

不得弱化 external-self、unknown-owner、requirement coverage或package-symbol rewrite validator。

## 验证与交付

使用共享target，先listing再执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test service_conformance -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test service_conformance
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test file_ir_execution_type_representation -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test file_ir_execution_type_representation
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked -p skiff-compiler
cargo fmt --all -- --check
git diff --check
```

预期 `14 / 2` 全部通过。写
`P5-F419D-suspension-compiler-current-fixture-repair-result.md`，记录exact commit/tree、protocol
mutation、PackageSymbol数据流、旧unknown断言删除、计数与边界。提交并保持clean；不merge/rebase/push。
若需要production修改则返回`TASK_SCOPE_EXPANDED`。
