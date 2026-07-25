# P5-F323 Eval fixture schema and heap closure

状态：Ready。

## 直接父节点

- combined probe finding wave：
  `P5-F320-representation-combined-probe-result.md`

父节点已证明representation targeted chain PASS。本任务只关闭完整eval第一次运行中精确归类的19个hydrated
schema fixture失败和1个跨heap错误断言；两个generic WebSocket source-inline失败另受用户设计决定约束，
不属于本任务。

## DAG位置与候选

- 节点：F320 finding wave机械fixture batch；与WebSocket设计分支及R0 core并行。
- 完成后解除完整eval复探针中的20/22 blockers；不改变representation或service error production结论。
- 证据基线：worktree创建时integration HEAD。Hydrated package admission API、Package schema identity或
  heap clone语义变化会使本证据失效。

## 写入范围

只允许test support/fixture：

- `runtime/eval/src/test_support.rs`
- `runtime/eval/src/assembly_execution/ordinary/tests.rs`仅必要caller适配
- `runtime/eval/src/assembly_execution/projection.rs`仅test module
- `runtime/eval/src/spawn_ops/canonical_tests.rs`
- `runtime/eval/src/test_effect_registry.rs`仅
  `typed_throw_clones_the_exact_carrier_into_the_request_heap`

优先在`test_support::link_package_fixture`一次修复19个共享失败，不逐个复制schema bypass。禁止修改
runtime production execution、linker/loader/linked-program、compiler、artifact model、WebSocket fixture、
representation tests、权威设计。

## 完成标准

### 1. Hydrated schema

- `link_package_fixture`为每个fixture package提供与
  `PackageArtifact.package_schema_index`的package id和identity精确一致的
  `PackageSchemaIndex`；
- 无公开schema record的fixture可使用严格空`types`，但不能使用伪package id、空identity或跳过admission；
- fixture若声明/引用public schema，必须提供真实index/records，不能用空index掩盖；
- 19个父节点列出的`MissingHydratedSchemaIndex`失败全部清除；
- 不放宽`HydratedPackageCode`、`SharedPackageLinkedImage`或service error index的production校验。

### 2. Heap断言

- 删除“不同`RequestHeap`中的数值handle必须不同”的错误假设；
- 用行为证明setup heap与request heap的节点彼此独立，clone后的outer/item exact carrier identity与payload保持；
- 可以让两个heap都合法分配`index:0,generation:0`，但对一个heap的读取/修改/释放不能借另一个heap的handle
  获得别名语义；
- 不改变`materialize_local_test_throw` production实现。

## 验证owner

先运行精确20个selector或其模块非零集合，再运行：

```bash
cargo test -p skiff-runtime-eval --lib -- --list
cargo test -p skiff-runtime-eval --lib --no-fail-fast
git diff --check
```

完整eval预期仍只剩父节点记录的两个generic WebSocket source-inline blocker。若出现新的独立失败，记录同一
代码状态下完整分类并停止；不得修改WebSocket设计或扩大范围。不运行workspace/root/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f323-eval-fixture-closure`
- branch：`codex/p5-f323-eval-fixture-closure`
- 风险：低至中，test-only机械closure；
- 新的一次性Agent，5分钟内先修共享helper和单一断言；提交并返回19+1矩阵及完整eval剩余失败；
- 不push、不承接WebSocket或acceptance。

