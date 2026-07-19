# P2-T05C13：Terminal Boundary Checker Refresh

## 目标

让compiler boundary checker精确冻结已验收的T04A–D terminal public shape，并按真实`#[cfg(test)]`模块可达性
排除test-only lowering fixture；不得通过通配allow-list或降低production policy让当前候选“变绿”。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”“Package-local ABI 与
Service ABI”及“Compiler 与 Projection 流水线”章节。

## 依赖与写域

- 依赖T04A–D、T05C12及T07 boundary 9-DENY finding；production Rust shape已由F09D验收，不在本任务修改。
- 独占`scripts/lib/compiler-terminal-public-shape.mjs`、`scripts/check-compiler-boundaries.mjs`与
  `scripts/tests/compiler-boundaries.test.mjs`。
- 不修改任何Rust、Cargo、artifact schema/identity、compiler fixture或其它checker。

## 完成态

1. frozen registry精确列出compiled fallible handoff：`ProjectionInputBuildError` declaration/re-export与
   `build_projection_input(...) -> Result<ProjectionInput, ProjectionInputBuildError>`；mutation self-test证明
   infallible旧签名、改名、额外字段/DTO或缺失error surface会失败。
2. projection-input frozen shape精确列出callable-signature DTO/re-export、
   `ExportPublicInstanceMethodProjection`、`ProjectionInput.callable_signatures`与唯一
   `canonical_package_public_path`函数；纯DTO规则只豁免这个被registry冻结的canonical helper，不允许任意free fn。
3. lowering forbidden-import扫描根据实际`#[cfg(test)] mod ...`模块及其文件后代排除test-only source；同一禁止
   import放入production module/file仍由`lowering_no_forbidden_imports`拒绝，不使用当前路径特例。
4. checker self-test/fixture同时覆盖current terminal PASS和上述代表性mutation FAIL；不新增legacy/compat关键词、
   通配allow-list、宽路径skip或已知违规ledger。

## 聚焦验收

- `node scripts/check-compiler-boundaries.mjs --self-test`。
- `node --test scripts/tests/compiler-boundaries.test.mjs`。
- `node scripts/check-compiler-boundaries.mjs`确认当前candidate零DENY，随后`git diff --check`；不运行其它T07 gate。

## 执行合同

- DAG：波次9n terminal checker checkpoint；完成后只解除T07 boundary复验和未执行的剩余gate。风险：高；
  结构gate owner，不改变production语义。
- worktree：`/Users/geek/workspace/skiff-p2-t05c13-boundary-checker`；分支：
  `codex/p2-t05c13-boundary-checker`；从`3b34570`创建。
- 启动后5分钟内完成第一次实际代码修改；修改前不跑测试或宽泛重研究。若current Rust shape与设计冲突，
  回报`TASK_NOT_EXECUTABLE`，不得用checker放宽代替production修复。
- 提交一个聚焦commit和自验收矩阵，不push。
