# P5-F445C-R1 Schema-index golden closure result

状态：`IMPLEMENTATION_PASS`。没有触发 `TASK_SCOPE_EXPANDED`。

本 leaf 证明 `declared_source_aliases_emit_only_canonical_file_ir_builtin_names` 中的
schema-index 漂移早于 F445C，且 F445C 的 package interface identity 修复没有改变该
projection。implementation 只将对应测试 golden 从旧值更新为两个历史点共同产生的 current
canonical value；没有修改 production、File IR golden、identity 算法或 timeout 实现。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| identity 修复前审计点 | `42edd1b5` | `8a124ce39574c7eff5b84405fd8659cf9ed82aff` |
| identity 修复后 / task base | `e48e7e11` | `9bf94c5e09c44be352de7884d9d6dc7359c8aa6c` |
| task dispatch | `960a8a7c` | `a09ea1d929de2dc54685c8501639552adb991d83` |
| test-only implementation | `784d2bff` | `050d39db13133dfa30c3c181ad688c154e1b8f92` |

implementation 精确修改：

- `compiler/tests/builtin_canonical_spelling.rs`

除此之外只新增本文 result。

## 2. 两个历史点的独立证据

在两个 detached 临时 worktree 中使用不同的 `CARGO_TARGET_DIR`，分别运行：

```text
cargo test -p skiff-compiler --test builtin_canonical_spelling \
  declared_source_aliases_emit_only_canonical_file_ir_builtin_names -- --nocapture
```

两次结果均在同一 schema-index 断言失败，且 expected / actual 完全一致：

```text
old golden:
skiff-package-schema-index-v1:sha256:9a92edc499c0e4f6e7b37a03418f87e923d09614fe029c7080735fa134959bf4

actual:
skiff-package-schema-index-v1:sha256:26b7640548d50a600c5e04e0b61851eb66d43b34bca65c26da99bacec2a7f577
```

| 审计点 | 结果 |
| --- | --- |
| `42edd1b5`，F445C 前 | actual 为 `26b764…f577` |
| `e48e7e11`，F445C 后 | actual 为 `26b764…f577` |

因此：

1. current canonical projection 在两个可独立编译的历史点稳定；
2. 漂移并非 F445C 引入；
3. F445C 前后 schema-index identity 不变，与父 result 的 artifact invariance 结论一致。

临时审计 worktree 和各自 target 在任务收尾时删除。

## 3. 实现与验证

唯一实现是把 `CURRENT_STD_SCHEMA_INDEX` 更新为已经由两个历史点共同证明的 actual value：

```text
skiff-package-schema-index-v1:sha256:26b7640548d50a600c5e04e0b61851eb66d43b34bca65c26da99bacec2a7f577
```

更新后，focused test：

1. 通过 schema-index 断言；
2. 到达并通过紧随其后的既有 File IR 断言
   `skiff-file-ir-v8:sha256:e62485ea5dcd42c0e4552db0e4271bc8bd573ca7478a09bfa238bd2183976cf8`；
3. 随后停在更后面的、非本任务所有的 package Local ABI golden：

```text
actual:
skiff-package-local-abi-v7:sha256:4e370158a4a654c55f0e086509368ebbdf34c5bfb818d5161aca18fcb62711ac

old golden:
skiff-package-local-abi-v7:sha256:a3923f5b29d9f1ac7373c679e6bcac4b13a1687ae29db4a98a1c73013509cc9e
```

该后续漂移不阻挡本 leaf 的 schema-index / File IR 证据，但仍会使整个 focused test 返回失败。
本任务没有更新 Local ABI、继续审计其来源或扩大写集；应由其 owner 单独判断。

其余 gate：

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

Cargo 只报告仓库既有 unused/dead-code warnings。

## 4. Scope 与禁令

- 没有修改 production projection、File IR golden、identity 算法、timeout 或 runtime。
- 没有启动 stable instance、router、runtime、telemetry、MongoDB 或任何 live workload。
- 没有访问 network。
- 没有派生子 Agent，没有 merge、rebase 或 push。
- implementation 与 result 分开提交。
