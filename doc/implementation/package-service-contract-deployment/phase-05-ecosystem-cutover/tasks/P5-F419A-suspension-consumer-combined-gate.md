# P5-F419A Suspension consumer combined gate

状态：Ready（N1 / N2 / N3 合流门禁，只读代码）。

## 直接父节点

- `P5-F417-suspension-compiler-inference-projection-result.md`
- `P5-F418-suspension-deployment-admission-result.md`
- `P5-F419-suspension-runtime-unified-boundary-result.md`

本节点不重新实现设计，只验证三个从同一 N0 checkpoint 派生的 consumer 在同一个精确代码状态上真实闭合。

## 精确候选与边界

- combined production candidate：
  `2b9d29eea9a65ab323240f1e6c34b3e3b29c7403`；
- tree：
  `fc6e7bfb05f4011eb4e0337944507ca3bc67d0cd`；
- accepted F415 ancestor：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时必须证明 candidate为HEAD ancestor且tree匹配，并证明 F415仍为ancestor。

production / test代码完全只读。唯一允许写入：

```text
本任务 result
```

不得修复失败、修改fixture、merge/rebase/push、访问stable/live或派子 Agent。任何失败都记录原始命令、
首错、归属与是否阻断 F420；不要把静态计数冒充实际 listing。

## 必跑门禁

所有 Cargo命令使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

先对每个 test selector运行对应 `-- --list`，再运行：

```bash
# F417 compiler
cargo test --locked -p skiff-compiler-core package_interface
cargo test --locked -p skiff-compiler-source callable_effects
cargo test --locked -p skiff-compiler-lowering suspend
cargo test --locked -p skiff-compiler-projection package_artifact
cargo test --locked -p skiff-compiler-compiled --lib
cargo test --locked -p skiff-compiler-contract --lib
cargo test --locked -p skiff-compiler --test service_conformance
cargo test --locked -p skiff-compiler --test file_ir_execution_type_representation

# F418 deployment
cargo test --locked -p skiff-deployment projection
cargo test --locked -p skiff-deployment storage
cargo test --locked -p skiff-deployment assembly

# F419 runtime
cargo test --locked -p skiff-runtime-capability-context execution_control
cargo test --locked -p skiff-runtime-request execution_budget
cargo test --locked -p skiff-runtime-model callback_projection
cargo test --locked -p skiff-runtime-eval assembly_execution
cargo test --locked -p skiff-runtime-native callback_adapter
cargo test --locked -p skiff-runtime-linker assembly
cargo test --locked -p skiff-runtime-loader runtime_assembly
cargo test --locked -p skiff-runtime-host assembly_admission

# Supplemental integration-risk probes
cargo test --locked -p skiff-artifact-identity public_instance
cargo test --locked -p skiff-runtime-eval --lib

# Combined compile
cargo check --locked \
  -p skiff-compiler \
  -p skiff-deployment \
  -p skiff-runtime-capability-context \
  -p skiff-runtime-request \
  -p skiff-runtime-model \
  -p skiff-runtime-eval \
  -p skiff-runtime-native \
  -p skiff-runtime-linker \
  -p skiff-runtime-loader \
  -p skiff-runtime-host

cargo fmt --all -- --check
git diff --check
```

可以把相同 `CARGO_TARGET_DIR` 作为环境变量一次导出，不能换到 stable或清理共享cache。

## 静态必须确认

1. production中不存在旧 requirement / protocol owner或访问：
   `BoundaryCancellationContract`、`BoundaryOperationContract.cancellation`、
   `BoundaryOperationContract.may_suspend`、
   `CallbackContractOperationProjection.may_suspend`。
2. concrete `Executable*`、`CallableMayEffects`、`PackageCallableSignature`与public/link exact equality仍在。
3. F415 mapping production链仍在，且 F419 13个initializer为 `4 / 3 / 4 / 2`。
4. unified runtime lane只有 `async_stream_cancel` 执行service；`ordinary.rs` 只有package-direct。
5. consumer可见的stream deadline测试实际列出并通过，不能只检查helper enum。
6. current positive producer只发出 Package v9 / Local ABI v7 / build v10 / protocol v5；真实legacy
   rejection字符串不计作残留。

## 判定与交付

只有全部 required selector、supplemental full eval、combined check、format/diff和静态检查都通过，才判定
`PASS` 并解除 F420。任何失败判定 `FAIL`，按最小 owner归类；不得自行修复。

写 `P5-F419A-suspension-consumer-combined-gate-result.md`，记录 exact commit/tree、每个listing与执行计数、
完整命令矩阵、stream deadline端到端证据、mapping与generation反搜、失败（如有）及 F420是否解除。提交
result并保持worktree clean；不 merge/rebase/push。
