# P5-F419B Suspension compiler integration fixture repair

状态：Ready。

## 直接父节点

- `P5-F419A-suspension-consumer-combined-gate-result.md`

父门禁已证明 production、combined compile与核心 compiler selectors正确；本节点只修复门禁暴露的三个
compiler integration fixture断点，不改变 suspension设计或 production。

## 精确起点与独占范围

- gate result commit：
  `087469235de2d1bb67965bce884b963d537c3f47`；
- failed production candidate：
  `2b9d29eea9a65ab323240f1e6c34b3e3b29c7403`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明三个commit都是HEAD ancestor。

唯一允许写入：

```text
compiler/tests/service_conformance.rs
compiler/tests/file_ir_execution_type_representation.rs
本任务 result
```

禁止修改production、model、identity、runtime、deployment、tooling、设计或其它fixture；不得派子 Agent、
merge/rebase/push、stable/live。

## 精确修复

1. `protocol_identity_tracks_semantics_but_not_diagnostic_text` 不得再访问已删除的
   `BoundaryOperationContract.may_suspend`。改用仍属于 code-free protocol shape 的真实语义变化证明
   protocol identity改变，例如参数/返回/stream/callback/value-plan中的最小合法shape mutation；保留
   diagnostic text不改变identity的正例。不得给contract恢复provider bit。
2. 两个 FileIR execution representation fixture当前把contract schema package与consumer package都写成
   `example.com/file-ir-execution-types`，触发正确的“external self reference未重写”拒绝。给contract
   schema seed使用独立canonical package id，并让contract requirement / resolved schema逐跳引用该id；
   consumer自身package id保持原值。不得弱化package-symbol validator或改期待的opaque execution
   representation。

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

预期实际数量为 `14 / 2`。写
`P5-F419B-suspension-compiler-integration-fixture-repair-result.md`，记录exact commit/tree、每个旧失败的
新证据、计数和边界。提交并保持clean；不merge/rebase/push。发现需要production修改则停止并返回
`TASK_SCOPE_EXPANDED`。
