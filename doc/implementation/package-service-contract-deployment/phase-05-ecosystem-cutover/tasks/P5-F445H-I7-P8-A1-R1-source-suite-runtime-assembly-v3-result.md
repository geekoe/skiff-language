# P5-F445H I7 P8 A1-R1 source-suite RuntimeAssembly v3 closure result

状态：

```text
COMPLETE
SOURCE_SUITE_V3_RECEIPT = PASS
DECISION_REQUIRED = NO
```

## 1. Input and root cause

冻结输入：

```text
baseline commit = f28ecd9a2099c575bfbe6e3aad40296d7157e559
baseline tree   = 1cd60da6c2c4379914d1abf15e3dd34b45e3bcbb
```

权威 current producer 和 consumer 事实：

- `artifact-model/src/schema.rs`只声明`skiff-runtime-assembly-v3`；
- `artifact-model/src/activation_lexical.rs`只接受
  `skiff-runtime-assembly-v3:sha256:<64 lowercase hex>`，v1/v2均为历史负例；
- `test-runner/src/package_service_host_fixture.rs`把compiler authoring receipt解析为
  `RuntimeAssemblyRef`后写入`baseAssembly`，真实输出正确为v3；
- `scripts/lib/skiff-source-test-suite.mjs`仍以v2正则二次校验该receipt，导致正确v3在runner启动前被拒绝。

这是确定性的source-suite harness drift，不需要修改schema、identity生成、compiler、runtime、Router或
A1 fixture。

## 2. RED and implementation

RED：

```text
commit = 5a60f63610c22ddf211cef38fe73c8b5e62de5c6
tree   = aace63281e598a73a912cdb66cb81811ac89d763
```

把直接receipt fixture切到真实v3，并增加历史v2负例后：

```text
node --test --test-name-pattern='package-service host receipt' \
  scripts/tests/skiff-source-test-suite.test.mjs
=> 1 discovered, 0 passed, 1 failed
=> Error: base assembly assemblyIdentity must be canonical
```

implementation：

```text
commit = 1f091b1690900c920fcb7312f762670742fa3ba9
tree   = 335e6509a638655b0856c50cf052664fff64f47e
```

`readPackageServiceHostFixtureReceipt`现在只接受current v3精确格式。直接测试固定：

- v3 lowercase 64-hex正例；
- v2历史负例；
- uppercase负例；
- 63位短hash负例。

没有v2 fallback，也没有接受任意版本。

## 3. Evidence

```text
node --test --test-name-pattern='package-service host receipt' \
  scripts/tests/skiff-source-test-suite.test.mjs
=> 1 passed

node --test scripts/tests/platform-source-transport-combined.test.mjs
=> 1 passed

node --check scripts/lib/skiff-source-test-suite.mjs
=> PASS

git diff --check
=> PASS
```

真实canonical链路使用`test-services/std`作为不依赖A1的最小registry entry，随后执行同一suite固定的Host
阶段：

```text
runCanonicalSkiffSourceTests({
  registry: [{ id: "std", root: "test-services/std" }]
})
=> std 11 passed
=> real Host preparer emitted
   skiff-runtime-assembly-v3:sha256:b3b39217...2bb6c1e8
=> receipt reader accepted it
=> package-service-host 4 passed
=> isolated Router/Runtime/Mongo stopped and temporary root removed
```

第一次以A1 registry运行的探针在进入Host preparer前被当前冻结baseline尚未闭合的receiver-method
lowering挡住；它没有产生本节点的receipt结论。改用std registry后，同一真实Host producer、reader、
runner和cleanup链路完整通过。

直接测试文件的完整执行另有一个baseline既存失败：
`F270 legacy overlay smoke debt remains an exact closed inventory`没有登记三个已经在baseline存在的P8 fixture
目录；其余9项通过。本节点没有修改该独立inventory owner。

## 4. Reverse search and follow-up classification

本节点owned路径中的v2只剩：

- `skiff-source-test-suite.test.mjs`中的明确历史负例；
- `platform-source-transport-combined.test.mjs`中属于另一个
  `encrypted-storage-live-contract` owner的输入。

零worktree搜索还发现以下独立script owner仍以v2为current正例：

```text
scripts/lib/package-service-authoring.mjs
scripts/lib/encrypted-storage-live-contract.mjs
scripts/lib/package-service-ecosystem-smoke-oracle.mjs
scripts/lib/package-service-i02-combined-oracle.mjs
及其直接tests/helpers
```

它们不参与本次source-suite receipt读取，修复会横跨activation/live/ecosystem owner，因此没有吞入本节点；
应由主Agent归为单独的terminal artifact consumer收敛任务。

## 5. Write set and handoff

实际写集：

```text
scripts/lib/skiff-source-test-suite.mjs
scripts/tests/skiff-source-test-suite.test.mjs
scripts/tests/platform-source-transport-combined.test.mjs
本task及result
```

保持NO-OP：

```text
artifact-model
artifact-identity
compiler
test-runner production
runtime
Router
A1 fixture
```

交付：

```text
branch   = codex/source-suite-runtime-assembly-v3-fix
worktree = /Users/geek/workspace/skiff-source-suite-runtime-assembly-v3-fix
```

交给`/root/phase05_integration_steward`串行集成和一级worktree/branch清理；本节点不merge、不push。
