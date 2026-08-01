# Rust 大型测试模块重构：line gate 结果

日期：2026-08-01

状态：completed

直接父节点：[`rust-large-test-module-refactor-stage.md`](./rust-large-test-module-refactor-stage.md)，唯一权威设计为
[`rust-large-test-module-refactor.md`](./rust-large-test-module-refactor.md)。本结果只覆盖 DAG 节点 D；后续 E/F/G
仍由父节点规定的 owner 完成。

## 基线与结论

- C baseline 精确为 commit `eb53d7c67cfaeecf5eaf74351d0d98690d0d56d2`，tree
  `05c7b5e2e413d31add48879fc4278fd4412963c4`；零 worktree 预检通过 `git rev-parse` 与 `git show`
  完成，没有 build、test 或 cache 写入。
- C 上共有 1392 个 tracked `.rs` 文件。按 checker 同一 `wc -l` 口径，唯一最大文件是
  `syntax/src/parser.rs`，4073 行；最大值并列数为 1。
- `scripts/check-rust-file-lines.mjs` 的 `MAX_FILE_LINES` 从 6533 精确降为 4073，同行
  `current maximum` 注释随之恢复真实；算法、消息、allowlist/exception 均未改变。
- 更新前后 Rust 计数清单完全相同，均为 1392 项，完整降序清单 SHA-256 均为
  `94dedba2c917a2badc418129cd355b7729dff0f7d4ffac6a613886c8fd0af821`。

## 计数方法与完整 Top 30

全量 tracked 清单使用以下可复现命令生成；排序先按行数降序，再按路径字节序升序：

```bash
git ls-files '*.rs' |
while IFS= read -r p; do
  n=$(wc -l < "$p" | tr -d ' ')
  printf '%7d %s\n' "$n" "$p"
done | LC_ALL=C sort -nr -k1,1 -k2,2
```

完整 Top 30 如下；第 1 名是阈值的唯一来源：

```text
4073 syntax/src/parser.rs
3151 compiler/source/src/type_resolution_model.rs
3130 compiler/source/src/type_resolution_model/tests.rs
3029 compiler/source/src/type_resolution_model/query.rs
3018 compiler/lowering/src/function_lowering.rs
2749 runtime/driver/eval/tests/support/executables.rs
2700 runtime/linked-type-plan/src/type_plan.rs
2683 runtime/driver/eval/tests/program_execution.rs
2630 syntax/src/parser/tests.rs
2549 compiler/lowering/src/source_file_lowering/tests.rs
2494 compiler/tests/package_imports.rs
2383 runtime/boundary/src/recoverable.rs
2376 compiler/source/src/expression_type_model.rs
2277 compiler/tests/runtime_slots.rs
2203 runtime/native/src/registry/tests.rs
2158 runtime/eval/src/eval_context.rs
2147 runtime/linker/src/assembly/tests.rs
2093 artifact-identity/src/package_artifact/tests.rs
2073 compiler/lowering/src/db_lowering.rs
2048 runtime/loader/src/runtime_assembly/tests.rs
2023 runtime/host/src/loader/assembly_admission/tests/execution/artifacts.rs
2000 runtime/linked-program/src/linked.rs
1953 runtime/eval/src/assembly_execution/service_error_channel/tests.rs
1936 compiler/source/src/type_resolution_model/shape_assignability.rs
1929 runtime/eval/src/assembly_execution/ordinary/tests/service_error_consumer.rs
1906 runtime/eval/src/assembly_execution/ordinary/tests.rs
1876 runtime/eval/src/error.rs
1863 runtime/model/src/recoverable.rs
1854 compiler/source/src/semantic/interface.rs
1840 compiler/source/src/expression_type_model/tests.rs
```

两个被重构领域不再是最大文件；其根文件仅声明模块，各领域/support 文件也均低于上述 4073 行最大值。

## tracked 与 checker 集合

checker 使用 `rg --files --glob '*.rs'` 取路径，再一次性调用 `wc -l`。对提交前不含任何额外 `.rs`
文件的工作树执行：

```bash
git ls-files '*.rs' | LC_ALL=C sort > /tmp/skiff-rust-line-gate-tracked.txt
rg --files --glob '*.rs' | LC_ALL=C sort > /tmp/skiff-rust-line-gate-rg.txt
cmp /tmp/skiff-rust-line-gate-tracked.txt /tmp/skiff-rust-line-gate-rg.txt
```

`git ls-files` 与 `rg` 均为 1392 项，`cmp` 退出码为 0，差集为空。最终提交后在 clean worktree 上再次执行
同一命令；该次 clean-state 结果随交接矩阵记录，若结果不一致则本节点不得交接。

## 验证矩阵

| 验证 | 结果 | 关键证据 |
| --- | --- | --- |
| C commit/tree 身份 | PASS | `eb53d7c6…` / `05c7b5e2…` |
| 更新前 tracked 全量计数 | PASS | 1392 项；最大 4073；清单 SHA-256 `94dedba2…` |
| 更新后 tracked 全量计数 | PASS | 1392 项；与更新前 `cmp` 相同；SHA-256 `94dedba2…` |
| tracked / `rg` path 集合 | PASS | 1392 / 1392；`cmp` 退出码 0；无差集 |
| `node scripts/check-rust-file-lines.mjs` | PASS | `Rust file line gate passed: 1392 files, limit 4073 lines.` |
| max−1 负向探针 | PASS | 临时 threshold 4072 时仅识别 `syntax/src/parser.rs: 4073 lines (limit 4072)` |
| `git diff --check` | PASS | 退出码 0 |
| 精确写集 | PASS | 仅 checker、line-gate leaf、line-gate result |

max−1 探针以临时 Node stdin 命令复用 checker 的 `rg`、`wc` 和解析口径，没有修改或提交 checker 算法。
按节点合同无需 Cargo、focused tests 或 full verify，本节点没有运行它们。

## 写集与停止条件

实际写集仅为：

- `scripts/check-rust-file-lines.mjs`；
- `doc/implementation/rust-large-test-module-refactor-line-gate-leaf.md`；
- `doc/implementation/rust-large-test-module-refactor-line-gate-result.md`。

未修改任何 `.rs`、A/B 文档、生产/测试源码、manifest、lockfile、schema 或配置；未新增依赖、算法、消息、
allowlist 或 exception。`TASK_SCOPE_EXPANDED` 与 `TASK_NOT_EXECUTABLE` 均未触发。本节点不 merge、push 或清理
worktree；最终 commit/tree 和 clean-state 复验由交接消息精确固定。
