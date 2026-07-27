# P5-F445C-R2 Package identity golden closure result

状态：`IMPLEMENTATION_PASS`。没有触发 `TASK_SCOPE_EXPANDED`。

本 leaf 证明 `declared_source_aliases_emit_only_canonical_file_ir_builtin_names` 中剩余的
package Local ABI 与 package build identity 漂移早于 F445C，且 F445C 的 package interface
identity 修复没有改变这两个 projection。implementation 只更新了两个测试 golden，没有修改
production、identity 算法、schema-index、File IR 或 timeout 实现。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| identity 修复前审计点 | `42edd1b5` | `8a124ce39574c7eff5b84405fd8659cf9ed82aff` |
| identity 修复后 / production base | `e48e7e11` | `9bf94c5e09c44be352de7884d9d6dc7359c8aa6c` |
| task base 修订 | `10726cef` | `ba5846733650d3f2df2420d717b70422b1795a4e` |
| test-only implementation | `52bdbdab` | `49c4118aadd4b3b27b3744f53160b5bb347739fa` |

implementation 精确修改：

- `compiler/tests/builtin_canonical_spelling.rs`

除此之外只新增本文 result。

## 2. 两个历史点的独立证据

在两个 detached 临时 worktree 中先应用同一份 R1 schema golden patch `784d2bff`，并分别使用
独立的 `CARGO_TARGET_DIR` 运行：

```text
cargo test -p skiff-compiler --test builtin_canonical_spelling \
  declared_source_aliases_emit_only_canonical_file_ir_builtin_names -- --nocapture
```

两个历史点依次产生完全相同的 actual：

| Identity | `42edd1b5`，F445C 前 | `e48e7e11`，F445C 后 |
| --- | --- | --- |
| package Local ABI | `skiff-package-local-abi-v7:sha256:4e370158a4a654c55f0e086509368ebbdf34c5bfb818d5161aca18fcb62711ac` | 相同 |
| package build | `skiff-package-build-v10:sha256:b3a2d0e8059cbad6f90c9e9dd48376e1d7c7a9c18de6063a60c2c24b8653a112` | 相同 |

在每个临时 worktree 中依次把前一旧常量替换为观测到的 actual 后，focused test 都完整通过，
因此没有隐藏的后续 identity 失败。临时审计 worktree 与各自 target 已删除。

这证明：

1. 两个 identity 在 F445C 前后稳定；
2. 漂移并非 F445C 引入；
3. 更新只是让测试基线跟随已经存在且稳定的 current canonical projection。

## 3. 实现与验证

唯一实现是更新：

- `CURRENT_STD_LOCAL_ABI` 为 `skiff-package-local-abi-v7:sha256:4e370158a4a654c55f0e086509368ebbdf34c5bfb818d5161aca18fcb62711ac`；
- `CURRENT_STD_BUILD` 为 `skiff-package-build-v10:sha256:b3a2d0e8059cbad6f90c9e9dd48376e1d7c7a9c18de6063a60c2c24b8653a112`。

正式 worktree 使用独立 target
`build/cargo-target-r2-final`，验证结果：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler --test builtin_canonical_spelling declared_source_aliases_emit_only_canonical_file_ir_builtin_names -- --nocapture` | PASS，`1 passed; 0 failed` |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

Cargo 只报告仓库既有 unused/dead-code warnings。

## 4. Scope 与禁令

- 没有修改 production projection、identity 算法、schema-index、File IR、timeout 或 runtime。
- 没有启动 stable instance 或执行 live workload。
- 没有访问 network。
- 没有派生子 Agent，没有 merge、rebase 或 push。
- implementation 与 result 分开提交。
