# P5-F445H I7 P8 A1-R2 terminal RuntimeAssembly v3 hard cut result

状态：

```text
COMPLETE
TERMINAL_CURRENT_CONSUMERS_V3 = PASS
LEGACY_V2_REJECTION = PASS
DECISION_REQUIRED = NO
```

## 1. Input and implementation

冻结输入：

```text
baseline commit = cc14290d2313e0db56eaf6d47eaf22bb545ae6dd
baseline tree   = 17eeff992aad7b0626ac0f38e324913147262d50
```

RED反向搜索在任务write set内发现22处把RuntimeAssembly v2当作current值的literal，覆盖authoring
activation、encrypted-storage live contract、ecosystem smoke/I02 oracle、helper、store corpus、native actor
fixture和Runtime README。

implementation：

```text
commit = 2bce7f3655ef9864174051aab7284e7a071b520a
tree   = f9efbb9ee226e9a6e93ddeb18b286774c7bb1fab
```

所有current consumer现在严格使用：

```text
schema = skiff-runtime-assembly-v3
identity = skiff-runtime-assembly-v3:sha256:<64 lowercase hex>
```

`package-service-authoring`和encrypted-storage receipt测试分别新增v2拒绝输入；没有v2 fallback，也没有
接受任意版本。producer、artifact schema、identity generation和其它generation保持NO-OP。

## 2. Evidence

聚焦Node测试：

```text
node --test \
  scripts/tests/package-service-authoring.test.mjs \
  scripts/tests/package-service-dev-sync.test.mjs \
  scripts/tests/encrypted-storage-live-harness.test.mjs \
  scripts/tests/platform-source-transport-combined.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs
=> 33 passed / 0 failed
```

native actor selector：

```text
cargo test -p skiff-runtime-native \
  all_four_actor_registry_routes_are_owned_external_waits
=> 1 passed / 0 failed
```

runtime wire/store corpus：

```text
node cross-system-fixtures/package-service-ecosystem/verify.mjs \
  --runtime-wire-self-test
=> ok:true
=> activationFrames 6
=> activationMutations 7
=> requestHeaders 3
=> requestMutations 114
=> requestRawCases 19
=> storeOperations 6
```

静态检查：

```text
node --check <10个触碰的.mjs>
=> PASS

git diff --check
=> PASS
```

预检列出的六文件组合命令实际得到`38 passed / 1 failed`。唯一失败是
`package-service-ecosystem-http-fixture.test.mjs`中的
`router compiler fixture uses split HTTP authoring and keeps dev timeout`：baseline测试仍要求`http.yml`
含已被当前authoring删除的`host: websocket-fixture.skiff.localhost`。本节点没有修改该测试或fixture，
失败也发生在RuntimeAssembly断言之前；这是独立的陈旧HTTP fixture expectation，按任务边界没有扩张修复。
同一命令中其余38项通过。

## 3. Reverse search

排除历史实现文档后，v2只剩以下明确负例：

```text
artifact-identity/src/runtime_assembly.rs
artifact-model/src/schema.rs
cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json
scripts/tests/skiff-source-test-suite.test.mjs
scripts/tests/package-service-authoring.test.mjs
scripts/tests/encrypted-storage-live-harness.test.mjs
```

前四项是既有legacy/mutation拒绝语料；后两项是本节点新增的hard-cut拒绝断言。active consumer中v2为零。
`doc/implementation/**`只保留历史执行记录，不作为current语义。

## 4. Write set and handoff

实际写集：

```text
cross-system-fixtures/package-service-ecosystem/ecosystem-store-cases.json
runtime/README.md
runtime/native/src/dispatch/prepared_tests/actor.rs
scripts/lib/encrypted-storage-live-contract.mjs
scripts/lib/package-service-authoring.mjs
scripts/lib/package-service-ecosystem-smoke-oracle.mjs
scripts/lib/package-service-i02-combined-oracle.mjs
scripts/tests/encrypted-storage-live-harness.test.mjs
scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs
scripts/tests/package-service-authoring.test.mjs
scripts/tests/package-service-dev-sync.test.mjs
scripts/tests/package-service-i02-combined.test.mjs
scripts/tests/platform-source-transport-combined.test.mjs
本task及result
```

交付：

```text
branch   = codex/terminal-runtime-assembly-v3-hardcut
worktree = /Users/geek/workspace/skiff-terminal-runtime-assembly-v3-hardcut
```

交给`/root/phase05_integration_steward`串行集成与清理。本节点未merge、未push、未运行live或stable instance。
