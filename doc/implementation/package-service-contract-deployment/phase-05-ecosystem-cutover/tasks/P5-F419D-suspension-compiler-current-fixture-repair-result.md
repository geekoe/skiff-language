# P5-F419D Suspension compiler current fixture repair result

状态：**COMPLETE**。两个过时 compiler integration fixture 已迁移到 current suspension /
Package nominal FileIR 终态；没有 production 修改或 scope expansion。

## 1. Exact checkpoint 与 ancestry

| 项目 | commit | tree |
| --- | --- | --- |
| task worktree start | `9efb9785deda3c170f0bc674a4c31e4ac0d18585` | `6036f8efda3e990492ff39d6bd733b039113eff8` |
| test-only implementation checkpoint | `679e10140e54e6793bdfeaaf49cd0adb337236eb` | `280b6a3f12f5b96a1bbabe70c4e243b85afe6712` |

启动时逐项执行 `git merge-base --is-ancestor <commit> HEAD`，以下四个 gate 均为 exit `0`：

```text
b7f7530d4b28b5c84e849a0ea2358c02ed435193
2b9d29eea9a65ab323240f1e6c34b3e3b29c7403
fc34744187ca7a89a29b839e16e4c5716e0e0235
7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d
```

implementation checkpoint 只修改：

```text
compiler/tests/service_conformance.rs
compiler/tests/file_ir_execution_type_representation.rs
```

## 2. Protocol identity current mutation

`protocol_identity_tracks_semantics_but_not_diagnostic_text` 不再访问已删除的
`BoundaryOperationContract.may_suspend`。code-free protocol shape 的合法 semantic mutation 改为：

```text
echo.return_value.ty: builtin string -> builtin bool
```

该 mutation 编译为合法 ServiceContract，并继续证明 protocol identity 改变。service、operation 与
type diagnostic text 的变化仍证明 identity 不变。

## 3. Package nominal FileIR 数据流

consumer package id 继续为：

```text
example.com/file-ir-execution-types
```

schema seed 使用独立 canonical owner：

```text
example.com/file-ir-execution-schema
```

同一 owner 逐跳贯通：

1. consumer `package.yml` 声明该 owner 的 direct package dependency，canonical alias 为
   `contractSchema`；
2. ServiceContract `PackageTypeRequirement.package_id` 为该 owner，并携带 seed 产生的 exact
   `PackageSchemaTypeId`；
3. `ResolvedPackageSchema` 的 owner 同为该 package id，alias 同为 `contractSchema`；
4. root PackageArtifact 的 direct `PackageRequirement` 保留 owner、alias、exact version 与
   dependency Local ABI；
5. executable parameter、return 与 `Array<...?>` nested leaf 全部投影为：

   ```text
   TypeRefIr::PackageSymbol {
     package: PackageRefIr::PackageId {
       package_id: "example.com/file-ir-execution-schema"
     },
     symbol_path: "Request",
     abi_expectation: None
   }
   ```

`Adapter.relay` 的 receiver 继续是本地 `TypeRefIr::LocalType { type_index: 0 }`；其 contract
parameter / return 则是上述 external PackageSymbol，两种角色没有混淆。

## 4. 旧 opaque assertion 删除与 fail-closed 保留

fixture 名称、helper 与失败信息已改为“preserve Package nominal execution identity”，旧
`opaque_unknown()` helper 和所有 positive builtin `unknown` assertion 已删除。

最终 executable wire 显式断言：

- 包含 canonical schema owner 与 stable key `Request`；
- 不包含该 owner seed 的 `PackageSchemaTypeId`；
- 不包含 `packageSchema`、`serviceSymbol` 或 builtin `unknown`。

external-self、unknown-owner、requirement coverage 与 package-symbol rewrite validator 均未修改或
绕过；fixture 通过真实 direct dependency closure 和现有 emission validation。

## 5. 验证

所有 Cargo 命令使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked -p skiff-compiler --test service_conformance -- --list` | PASS；实际 `14 tests / 0 benchmarks` |
| `cargo test --locked -p skiff-compiler --test service_conformance` | PASS；`14 passed / 0 failed` |
| `cargo test --locked -p skiff-compiler --test file_ir_execution_type_representation -- --list` | PASS；实际 `2 tests / 0 benchmarks` |
| `cargo test --locked -p skiff-compiler --test file_ir_execution_type_representation` | PASS；`2 passed / 0 failed` |
| `cargo check --locked -p skiff-compiler` | PASS；只有既有 advisory warnings |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

汇总：实际 listing `14 / 2`，execution `14/14 + 2/2 = 16/16`。

## 6. 边界

没有修改 production、validator、runtime、deployment、tooling、设计或其它 fixture；没有派子
Agent，没有 merge/rebase/push，没有访问 stable/live、instance 或 watch registry。
