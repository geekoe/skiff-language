# Phase 8: production hard cutover and deletion

状态：planned；依赖Phase 7 complete

## 1. 目标

把所有production ingress切到verified deployment image/VM，并物理删除tree artifact/evaluator、
`RuntimeAssembly`/generation和迁移fallback。阶段验收关注“不可达且不存在”，不是只把旧路径关掉。

## 2. 交付物

1. Compiler只生成新bytecode schema；package/deployment identity和store不再写tree executable body。
2. Gateway、service operation、package direct、Actor、durable task、callback和stream ingress全部进入同一
   loader/linker/verifier/VM路径。
3. 删除old artifact reader、tree linker/program DTO、`runtime/eval`执行入口、legacy projection/engine switch、
   assembly/generation admission和test-only evaluator。
4. Tests使用可注入artifact store，但走production loader/linker/verifier/VM；无test-only assembly admission。
5. 删除stale runtime/router/scripts/README/AGENTS术语与control flow，更新crate DAG和subject registry。

## 3. 验收

### 3.1 非Live完整gate

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only router
node scripts/verify.mjs --only test-runner
node scripts/verify.mjs --only tooling
node scripts/verify.mjs --only skiff-tests
node scripts/verify.mjs --only checks
pnpm test
git diff --check
```

Cargo仍串行。Full `pnpm verify`留给Phase 9，但本阶段不能跳过上述production subject和完整non-Live tests。

### 3.2 Reverse-search与结构证明

至少执行并逐项处置：

```bash
rg -n 'RuntimeAssembly|ActiveAssembly|AssemblyExecutionImage|assembly generation' \
  artifact-model artifact-identity compiler deployment runtime router test-runner scripts
rg -n 'RuntimeExecutionProjection::Legacy|LegacyExecution|tree evaluator|fallback.*evaluator' \
  compiler runtime router test-runner scripts
rg -n 'async_recursion|LinkedStmtIr|LinkedExprIr|ExecutableBody' \
  runtime compiler artifact-model test-runner
rg -n '__skiff/activate-assembly|RuntimeConfigSnapshot|expected-generation' \
  runtime router scripts AGENTS.md doc/overview.md runtime/README.md router/README.md
```

目标是production代码零命中。测试若保留malformed legacy tombstone bytes，必须在result ledger逐项解释且不能包含
reader/evaluator。历史implementation文档命中不构成代码例外，但当前操作文档必须更新。

还必须证明：

- 用旧schema、旧assembly record或legacy engine selector启动/发布时稳定拒绝；
- production binary中不存在可达tree evaluator symbol/registration；
- test fixture损坏bytecode时在同一production validator/verifier失败；
- crate DAG没有让VM依赖compiler/eval或让host adapter反向拥有VM内部类型。

### 3.3 强制 Live

```bash
node scripts/verify.mjs --only router-live:http
node scripts/verify.mjs --only router-live:ws
node scripts/verify.mjs --only router-live:actor
node scripts/verify.mjs --only durable-task-e2e-live
node scripts/verify.mjs --only router-live:agine
node scripts/verify.mjs --only router-live:clean-host
```

所有manifest必须只有VM schema/engine，fallback和legacy counters/fields不存在而不是为零。Clean-host rehearsal从空
cache/store初始化、发布、lazy-load、执行和关闭，不能依赖开发机遗留artifact。

## 4. 停止条件

- 旧reader/evaluator仅被feature flag、环境变量或dead-code属性隐藏。
- tests继续手工构造linked tree program或绕过verifier。
- 为兼容旧artifact/version保留try-old path。
- Phase 8首次发现并实现新的语言、boundary、Actor或memory语义。

## 5. Handoff

Phase 9只接受production legacy reverse-search零命中的Phase 8 complete checkpoint。发现功能缺口必须重开其
原owner阶段，不在release gate中临时补fallback。
