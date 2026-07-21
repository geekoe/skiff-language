# P5-F04：Test Infrastructure Integration Repair

## 输入、组合方式与写入范围

- 输入：D04冻结设计、T05 candidate `f8ad689`、包含T02–T04/F03A的当前integration。主integration owner先在
  replacement task worktree组合candidate；原T05 Agent只增加一个repair commit，不merge/push。
- 五分钟内先做WS authoring与canonical store/base assembly API探针，再产生真实code edit；若WS production
  projection不可构造，报告exact compiler blocker，不得改artifact。
- D07确认真实consumer另受compiler/source callable-effects阻断；F04不得绕过或修改compiler。只有F06合流并
  通过R06后，F04才在该exact checkpoint恢复同一fixture的isolated正例与最终接收。
- 独占`test-runner/**`、`runtime/package-test/**`、test-only ecosystem fixture、`scripts/skiff.mjs`、
  `scripts/lib/**`中isolated test/verify直接caller及对应tests。可删除旧live/doubles caller；不改compiler
  stream、Router、Runtime host/driver/shared wire或root lock。

## 完成态

1. Package source只编译自身；package/contract dependencies从canonical store exact typed refs解析。按需读取
   `--base-assembly`完整closure，并把test overlay/implementation投影成新的test-owned deployment/assembly；
   provider/config/state/resource/capability没有唯一binding时fail closed。
2. 一个真实consumer正例经production authoring、linker、Interpreter/Host执行helper package callee的同heap mutation，
   再通过`InProcessBoundary`调用provider，断言最终detached业务结果；direct test不得导入或手调mutation primitive。
3. D04 CLI表在Node wrapper与Rust runner完全一致。non-live目标只由isolated harness注入，live六元目标参数完整；
   F04-owned caller/help/verify registry/plan/tests中的删除选项零命中，不保留silent no-op/deprecated alias。
   extra-review发现的encrypted-storage live harness同时依赖legacy service.yml/reload/service selector，冻结为T06
   terminal migration，不在F04做不完整flag替换。
4. config/state只由base assembly中test-owned ServiceDeployment拥有；旧ambient config、effect double registry、
   synthetic service/package mock语义明确退役并删除。测试需要mock service时发布真实contract/package/deployment。
5. smoke删除server-stream bridge与PackageArtifact重签，改为canonical WebSocket A连接→activate B→新请求/连接到B→
   旧连接receive仍pin A后自然结束；self-test只模拟状态机，real smoke不复制production wire/parser。
6. `canonical_fixture.rs`删除或成为薄re-export；discovery、store publication、assembly projection、runtime execution
   各有聚焦模块。`runtime/package-test/tests/support`可保留聚焦test corpus，但实际执行helper独立。
7. F03C deferred callsites与新`PackageTestRuntimeBuilder`接口形成exact handoff；F04自身两crate通过，root lock恢复。

## 验证

```bash
cargo test -p skiff-test-runner --test package_service_contract_deployment
cargo test -p skiff-runtime-package-test --test package_artifact
cargo test -p skiff-runtime-eval package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation
node --test \
  scripts/tests/package-service-authoring.test.mjs \
  scripts/tests/package-service-store.test.mjs \
  scripts/tests/skiff-test-cli.test.mjs \
  scripts/tests/isolated-test-runtime.test.mjs \
  scripts/tests/verify.test.mjs \
  scripts/tests/verify-live-registry.test.mjs
node scripts/run-package-service-ecosystem-smoke.mjs --replicas 2 --self-test
git diff --check
```

`runtime-host`在T05 API删除后有已记录且仅位于F03C scope的编译断链；F04不得为运行host test添加兼容层或
修改host。F03C迁移完成后由其聚焦gate与I02证明`InProcessBoundary` production Host路径；F04自己的真实
service正例通过canonical isolated stack取得最终Host结果。

另反搜D04删除参数、`enable_ecosystem_smoke_server_stream`、production artifact identity mutation、手调mutation
primitive及旧aggregate。回报base→overlay→test assembly数据流、CLI disposition、真实执行结果、模块职责、
exact source/commit/tree与clean/lock。原接收reviewer只窄复验六个blocker。

## D07 / F06 handoff

F04在R06前保留负例，精确断言service-calling test callable的projection reasons；R06 PASS后必须使用正常source
构造fresh helper Box → package direct mutate → detached payments contract call，provider按输入分支返回常量，并
通过exact helper PackageBinding、service selector、base config与isolated Host ingress断言最终
`provider-observed-helper-mutated`。旧负例不能作为完成态，既有eval/Host focused test也不能替代该最终结果。

## Terminal live-harness handoff

`scripts/lib/encrypted-storage-live-harness.mjs`、`scripts/check-db-encrypted-storage-live.mjs`及
`runtime/encrypted-storage-live/**`是F04 reverse-search唯一允许的deferred deleted-flag/legacy caller，owner为T06。
F04必须修复`skiff-source-test-suite.mjs`等其余直接caller，并在回报中列出该exact exception；不得把它泛化为
legacy compatibility。`AGENTS.md`与`doc/architecture/test-runner-runtime-isolation.md`的canonical改写也归T06。

`verify runtime-live`的新CLI parser/plan字段与拒绝矩阵归F04，但现有`runtime/live-tests/**`只有legacy
`service.yml`且需要config/DB/file/http capability与base assembly，完整语义fixture也归T06。F04不得用孤立
`package.yml`或synthetic base assembly伪装可执行live phase；回报必须把它列为第二个exact terminal owner。
