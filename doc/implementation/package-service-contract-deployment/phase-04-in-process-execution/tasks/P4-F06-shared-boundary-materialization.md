# P4-F06：Shared Boundary Materialization Planner

## 权威输入、风险与证据状态

- 执行输入：R02在`ee1609c`的blocking issue 2及package-direct非阻塞证据缺口；T04同步lane拥有完整canonical
  parameter/return/typed-error planner，T05 async lane复制了parameter/return且漏掉typed error。
- 风险/验收组：高风险跨lane error/heap语义；由R02复验，不直接解锁Wave 3。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：T04–T06合流及R02 FAIL；可与F07并行。
- 解锁：F08。
- branch：`codex/p4-f06-shared-boundary-materialization`。
- worktree：`/Users/geek/workspace/skiff-p4-f06-shared-materialization`。
- 五分钟内真实edit；原T04 owner执行。不得把service executor、stream/callback lifetime或legacy program带入共享planner。

## 写入范围与完成态

- 在`runtime/eval/src/assembly_execution`新增lane-neutral canonical materialization/planner模块；T04 ordinary lane改为
  薄consumer。可修改owned ordinary tests/host ordinary lane，不修改async/stream/callback实现。
- planner直接消费operation descriptor的parameter/return/error `ContractTypeRef`、boundary schema与value plan，统一
  preflight、caller/provider hooks、fresh-heap detached materialization与错误分类。
- 对success、declared typed business error、undeclared typed throw、payload shape/schema/plan mismatch暴露明确API；
  sync与后续async consumer不能各自重写分类。
- `package_direct_same_heap`改为通过真实canonical package executable/dispatcher执行mutable object，证明same handle、
  alias与callee mutation caller可见；不再手工clone handle冒充executor证据。
- 不扩大已经很长的`ordinary.rs`；提取后应只保留ordinary call orchestration与聚焦测试。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-eval ordinary_in_process
cargo test -p skiff-runtime-eval service_error_boundary
cargo test -p skiff-runtime-eval package_direct_same_heap
cargo test -p skiff-runtime-host typed_execution_ordinary
git diff --check
```

四个过滤器必须非零PASS；不得运行完整runtime gate。

## 回报

提交一个clean commit，回报共享planner API、sync consumer diff、error分类矩阵、真实package-direct证据与命令结果。
