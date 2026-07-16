# P1-T01：Identity Module 与 Canonical JSON Foundation

## 目标

把通用 canonical JSON leaf 与 artifact identity owner 从单个 2460 行文件中拆开，同时保持现有
identity语义不变，为后续 nominal/package identity任务提供可维护的唯一入口。

## 依赖与 worktree

- 无前置代码任务；从 Phase 01 文档 checkpoint 建 task worktree。
- 建议 branch：`codex/package-service-p1-t01-identity-foundation`。
- 本任务提交后由主 Agent 合入 phase integration branch；不得直接合并 `main`。

## 完成态

1. 新增纯 leaf crate `skiff-canonical-json`，只提供 JSON key ordering、number normalization和canonical
   bytes；不知道任何 artifact schema、prefix或hash算法。
2. `artifact-identity` 按职责拆成至少 canonical/framing、file-ir、package/publication/operation、
   legacy-service/runtime-program、package-test、error 模块；`lib.rs` 只声明模块和稳定re-export。
3. `compiler/core/src/json_utils.rs` 与 `runtime/linker/src/json_utils.rs` 中语义完全相同的canonical实现
   委托 leaf crate。仅排序但不规范number的 helper必须保留不同名字和测试，不能错误合并。
4. `scripts/check-artifact-identity-single-source.mjs` 按 crate owner和public API检查，可接受多模块布局；
   扫描所有production Rust实现，不再硬编码canonical定义必须位于`artifact-identity/src/lib.rs`。
5. 新crate在workspace、crate DAG和`verify-rust-subjects`中归入恰好一个foundation subject。
6. 所有既有identity bytes、prefix和golden保持bit-identical；本任务不修preimage遗漏。

## 写入范围

- 根 `Cargo.toml`、lockfile以及新增 canonical-json crate。
- `artifact-identity/src/**` 的机械模块拆分。
- `compiler/core/src/json_utils.rs`、`runtime/linker/src/json_utils.rs`及其直接测试。
- `scripts/check-artifact-identity-single-source.mjs`、crate DAG/verify subject声明。

不要改 artifact DTO、package identity字段矩阵、effect model、router或service runtime语义。

## 验证

```bash
cargo fmt --all -- --check
cargo test -p skiff-canonical-json -p skiff-artifact-identity -p skiff-compiler-core -p skiff-runtime-linker
node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-compiler-crate-dag.mjs
git diff --check
```

必须增加golden/parity测试证明模块拆分前锁定的代表性File IR、package、publication operation、service
unit、runtime program和package-test identity没有改变。

## 自验收与回报

用 `rg` 证明production中canonical JSON实现只剩leaf owner；列出仍保留的非canonical sort helper及理由。
提交自验收矩阵、命令结果、worktree/branch/commit。测试通过不等于完成；`artifact-identity/lib.rs`仍
包含各identity具体projection即为FAIL。
