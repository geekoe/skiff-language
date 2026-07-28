# P5-F445H-I7-D7 Stateful package diamond result

状态：

```text
PASS
D7_COMPLETE=YES
STATEFUL_DIAMOND_BLOCKER=CLOSED
DECISION_REQUIRED=NO
BLOCKING_ISSUES=0
```

## 1. Exact input and implementation

| 项 | 值 |
| --- | --- |
| baseline commit/tree | `5e87d1ce3c3461e5687564807afea9db4943ba46` / `c9481fc7859919199ac84e6839b07847779fce02` |
| RED fixture commit/tree | `e9f52c07a9070aaf2afb2bc05c13434dc32e9873` / `deeff30326e4fd78c1c49a5beaaffd1b689fcb00` |
| implementation commit/tree | `fe72fb38f14cd9612f18515d7a5e12166770f1d4` / `37ca9cbc8902531052792fdd5ba5f00f9e8bd513` |
| branch | `codex/p5-f445h-i7-d7-stateful-diamond` |

Production写集只有artifact-model collection mapping helper、runtime loader graph validation和Runtime Host
active assembly DB metadata构造。测试写集只有Host full-chain fixture和test-runner package assembly
fixture。没有修改compiler、manifest、deployment resolver、Router、Internals、P3 DB target consumer或外部
package。

## 2. RED to GREEN

测试先提交AIHub形状：

```text
test root ───────────────> stateful C
    └──> subject B ──────> stateful C
```

在撤下production修改的F415实现上，empty/identical正例真实失败：

```text
multiple active collection projections
```

错误同时列出root direct edge与subject transitive edge，证明fixture命中了M2记录的同一owner。恢复实现后：

- empty/identical：PASS；
- non-empty/identical：PASS；
- reverse canonical link order：PASS；
- same exact build/different resolved mapping：fail closed；
- distinct build/same physical target与dependency/root collision：既有负例继续PASS。

## 3. Canonical merge owner

`CanonicalActiveCollectionProjection`是artifact-model中的非序列化runtime comparison value，不是artifact
DTO。它包含：

- 完整resolved source→target `BTreeMap`，因此missing、empty和explicit identity只按最终语义比较；
- activation唯一database namespace；
- exact `PackageBuildId`作为registry key，继续拥有不可变code和DB metadata facts。

Loader先解析每条edge，再比较candidate。相同candidate只保留第一份effective target owner；不同candidate
明确拒绝。Host使用同一helper，并只为第一份identical candidate生成`DbMetadataIr`。第二条edge没有被盲目
忽略：只有完整candidate比较相等后才合并。

Root collection继续单独拥有identity projection；dependency与root即使build相同也通过target/owner collision
fail closed。不同build指向同一target仍由activation target owner表拒绝。

## 4. Test-runner and recovery receipt

test-runner现有transitive-store fixture增加test root对leaf的direct dependency。最终assembly断言：

- leaf有两个真实incoming package links；
- leaf只有一个exact code slot；
- state requirement仍只形成一个case-scoped database namespace。

Host正例同时覆盖link顺序反转；既有mapping test覆盖committed recovery/reload前后DB metadata byte-for-byte
相同。新diamond正例确认provider只收到一份leaf metadata，未重复创建metadata/state owner。

## 5. Verification

| Gate | 结果 |
| --- | --- |
| artifact-model full locked suite | `181 passed / 0 failed` |
| runtime-loader full locked suite | lib `17/17`，integration `2/2` |
| runtime-host locked lib suite | `331 passed / 0 failed` |
| test-runner package-service integration | `28 passed / 0 failed / 1 ignored` |
| focused artifact canonical projection | `3 passed / 0 failed` |
| focused Host stateful diamond | `2 passed / 0 failed` |
| required four-package `cargo check --locked` | PASS；只有既有warnings |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |
| forbidden owner / generation reverse search | PASS |

当前generation保持PackageArtifact v9、Package build v10、Package Local ABI v7和RuntimeAssembly v3；没有
schema/wire bump、dual read、fallback、stable/live/network/Mongo/OAuth/browser或push。
