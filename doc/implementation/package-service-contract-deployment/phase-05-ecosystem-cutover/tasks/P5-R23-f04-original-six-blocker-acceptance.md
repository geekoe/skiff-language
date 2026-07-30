# P5-R23：F04 Original Six-blocker Acceptance

## 角色、权威范围与前置

使用未参与T05/F04/F04A/B及其D/F/R/H/I/G收敛链、R22、replacement I16或G16E的全新只读Agent。
原六项的唯一权威来源是P5-T05“接收审查记录”；D04/F04/F04A只冻结修复解释和动态前置，不得把后续environment、
readiness或Gate evidence问题增加为第七项。23个旧Host package-test编译错误也不属于原六项。

R23只能在同一exact candidate上的replacement I16 PASS与G16E PASS后启动。G16E是唯一完整动态最终结果前置，不是六项
之一，也不自动给F04 verdict；必须提供v6/full、`fullProbeRuns:1`、Host code0、std `11/11`、Host `1/1`、唯一exact
test name、`provider-observed-helper-mutated`、完整cleanup/ownership及`stableOperations:0`。G16D永久保持FAIL。

只给`R23 PASS`或`R23 FAIL`，不修改/提交、不运行I16/full/Host/`run-skiff-tests.mjs`、真实smoke、dependency install、
runtime或stable，不复验R22/F22A，不作R02或阶段verdict。HEAD/tree/lock/status前后必须逐字一致，写入数、提交数和stable
操作数均为0。PASS只关闭F04 receive并解锁F05；不直接解锁F03B/F03C、I02、R02或阶段完成。

## 原六项与唯一窄证据

每项命令只运行一次，任一失败立即`R23 FAIL`，不得修复或重试。

1. **canonical store/provider/base assembly接通真实结果**

   ```bash
   cargo test --locked -p skiff-test-runner \
     --test package_service_contract_deployment \
     contract_dependency_is_loaded_from_a_typed_pointer_and_record -- --exact
   ```

   要求`1 passed / 0 failed / 0 ignored`，并消费而非重跑G16E最终Host证据。

2. **package-direct执行真实callee并暴露same-heap mutation**

   ```bash
   cargo test --locked -p skiff-runtime-eval --lib \
     assembly_execution::ordinary::tests::package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation \
     -- --exact
   ```

   要求`1/0/0`；不得导入或手调mutation primitive冒充callee执行。

3. **Node wrapper与Rust runner CLI单一合同**

   ```bash
   cargo test --locked -p skiff-test-runner \
     --test package_service_contract_deployment \
     platform_source_context_contract -- --exact
   ```

   要求`1/0/0`；同时只读复用F20B Node `5/5`和R20B matched=1，前提是`scripts/skiff.mjs`与对应测试blob未变。

4. **base assembly与真实config/provider owner**

   ```bash
   cargo test --locked -p skiff-test-runner \
     --test package_service_contract_deployment base_assembly -- --test-threads=1
   ```

   必须精确匹配两个测试并得到`2 passed / 0 failed`：owner bindings正例与missing-base fail-closed。

5. **无artifact rewrite或synthetic stream bridge**

   ```bash
   cargo test --locked -p skiff-test-runner \
     --test package_service_contract_deployment \
     ecosystem_fixture_has_no_artifact_rewrite_or_synthetic_stream_bridge -- --exact
   ```

   要求`1/0/0`；并在F04-owned source中反搜以下旧owner命中均为0：

   ```text
   enable_ecosystem_smoke_server_stream
   assign_package_artifact_identities
   boundary_projections.insert
   ```

   D05交给F05的真实WebSocket authoring不是F04 blocker。

6. **fixture职责已拆分**

   只读结构检查必须同时满足：

   ```text
   canonical_fixture.rs中的`pub use crate::`：4
   canonical_fixture.rs workflow函数定义：0
   canonical_store/package_test_assembly/runtime_execution/test_discovery：4/4存在
   上述旧smoke mutation owner：0
   ```

   执行extra-review；文件长度本身不是finding。`canonical_fixture.rs`必须只是四个聚焦owner的薄re-export。

## 证据复用、失效与交付

可复用但不得代替本六项命令的证据：P26S helper projection/assembly `1/0/0`、F20B/R20B CLI、R08 package-artifact
`5/5`、D05 WebSocket handoff、P27R/R21C及F21 parser/marker；reviewer须逐blob确认其相关表面自各锚点后未变。

不得复用：G16D/旧I16、旧R21整体Gate结论、F04A早期Host结果，或仅为合同而非result ledger的R16。candidate/tree/lock、
F04-owned runner/fixture/store/assembly/eval/smoke、G16E/I16 evidence或六项命令测试任一变化会使R23失效。

回报exact candidate/tree/lock、G16E/I16 SHA与digest、六条命令exact argv/匹配数、结构/反搜计数、复用blob身份、clean
before/after、extra-review、blocking findings及write/commit/stable计数。R23与I16/G16E必须在不改变tracked candidate的
连续冻结窗口内完成；不得在三者之间提交result文档。
