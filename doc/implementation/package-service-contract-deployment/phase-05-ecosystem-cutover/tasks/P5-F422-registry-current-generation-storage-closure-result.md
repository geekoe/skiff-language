# P5-F422 Registry current-generation storage closure result

状态：scoped implementation与真实Registry runtime测试已闭合；canonical组合入口被写入范围外的旧
receipt oracle阻断。

```text
TASK_SCOPE_EXPANDED
```

本文不写`REGISTRY_STORAGE_CURRENT_PASS`。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| Skiff production/toolchain checkpoint | `1f289d8116f90448421566630798d54922c712eb` | `203c01a72908356fec4cc2ead75efbfc1bf32b65` |
| Skiff task/result checkout | `1e4e334e71de128879ebd6adfd8b13583d9a079a` | `a6865240003d1248511d9552be99940d9ba98a12` |
| skiff-packages start | `77eac6611e790ed06a6c381379467c08a9391b0a` | `c151e89ed9976975d9316945ee2f171b4ac590aa` |
| skiff-packages scoped implementation/tests | `519dc3d23396f67b919488c5612aea4c0087708e` | `d39c43c9a4c3dabaa287c01c44521b8d156cff8e` |

Skiff task checkout相对production/toolchain checkpoint只新增本任务文件。implementation分支为
`codex/p5-f422-registry-storage`，result分支为
`codex/p5-f422-registry-storage-result`。

## 2. Scoped implementation

`registry/immutable_store.skiff`的put/read validator已经精确迁到：

| record | schema | identity |
| --- | --- | --- |
| PackageArtifact | `skiff-package-artifact-v9` | build `skiff-package-build-v10:sha256`；Local ABI `skiff-package-local-abi-v7:sha256` |
| ServiceContract | `skiff-service-contract-v5` | protocol `skiff-service-protocol-v5:sha256` |
| ServiceDeployment | `skiff-service-deployment-v2` | `skiff-deployment-artifact-v2:sha256` |
| RuntimeAssembly | `skiff-runtime-assembly-v2` | `skiff-runtime-assembly-v2:sha256` |

没有加入任意prefix、dual-read、dual-write或历史兼容。启动审计证明
`registry/pointer_store.skiff`没有旧generation硬编码，因此按任务合同保持零修改。

两份动态测试的positive fixture全部使用current generation。新增的明确负例分别独立构造旧
PackageArtifact schema、package build、Local ABI、ServiceContract schema和service protocol；
旧prefix不会被当作positive fixture使用。旧测试对`catch`结果的
`.exception.error.tag`普通成员访问也已机械删除；current runtime把request-local exception作为
opaque carrier，仓内canonical测试只通过`catch<T>.tag == "err"`验证typed throw。

## 3. 真实immutable与pointer矩阵

直接执行组合入口被第4节的Node receipt oracle短路后，独立执行其原定runtime子命令：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
  node scripts/test-packages.mjs registry
```

真实结果为9 discovered、9 executed、9 pass、0 fail、0 skip。

| record | immutable put | same-content replay | read | 结果 |
| --- | --- | --- | --- | --- |
| PackageArtifact | executed | executed | executed | PASS |
| ServiceContract | executed | executed | executed | PASS |
| ServiceDeployment | executed | executed | executed | PASS |
| RuntimeAssembly | executed | executed | executed | PASS |

| pointer | initial CAS | second CAS | current read | ascending history | 结果 |
| --- | --- | --- | --- | --- | --- |
| PackageArtifact | sequence 1 | sequence 2 | sequence 2 | `[1, 2]` | PASS |
| ServiceContract | sequence 1 | sequence 2 | sequence 2 | `[1, 2]` | PASS |
| ServiceDeployment | sequence 1 | sequence 2 | sequence 2 | `[1, 2]` | PASS |
| RuntimeAssembly | sequence 1 | sequence 2 | sequence 2 | `[1, 2]` | PASS |

真实负例：

| 负例 | executed | 结果 |
| --- | ---: | --- |
| same PackageArtifact identity、different content | 1 | typed `RegistryError`，PASS |
| malformed RuntimeAssembly identity | 1 | typed `RegistryError`，PASS |
| PackageArtifact旧schema v7 | 1 | typed `RegistryError`，PASS |
| package build旧prefix v8 | 1 | typed `RegistryError`，PASS |
| Local ABI旧prefix v6 | 1 | typed `RegistryError`，PASS |
| ServiceContract旧schema v4 | 1 | typed `RegistryError`，PASS |
| service protocol旧prefix v4 | 1 | typed `RegistryError`，PASS |
| PackageArtifact pointer CAS expected mismatch | 1 | current/history未写入，PASS |
| RuntimeAssembly candidate release与key mismatch | 1 | rejected，PASS |
| history limit `0` | 1 | rejected，PASS |

test-runner已经越过F381的
`package-test ingress is not yet migrated to deployment gateway entries`。本次真实流程完成std
bootstrap、Registry publish、20项available operation、isolated Router/runtime/Mongo启动、
assembly activation和9项execution；没有出现新的Skiff production blocker。

## 4. 三类测试实际计数与scope blocker

| 测试类 | 命令 | discovered | pass | fail | skip |
| --- | --- | ---: | ---: | ---: | ---: |
| Registry source Node | `npm run test:registry-source` | 6 | 6 | 0 | 0 |
| Registry receipt Node | `SKIFF_ROOT=... npm run test:registry-receipt` | 1 | 0 | 1 | 0 |
| Registry service runtime | `SKIFF_ROOT=... node scripts/test-packages.mjs registry` | 9 | 9 | 0 | 0 |

任务指定的canonical命令：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration npm run test:registry
```

实际exit `1`。Node阶段共7 discovered、6 pass、1 fail、0 skip；`&&`因此没有自动进入runtime
阶段。first failure为：

```text
scripts/registry-service-receipt.test.mjs:123
actual   skiff-package-artifact-v9
expected skiff-package-artifact-v8
```

同一范围外positive receipt oracle还冻结了四个非current expectation：

```text
line 126  skiff-package-build-v9:sha256
line 130  skiff-package-local-abi-v6:sha256
line 179  skiff-service-contract-v4
line 183  skiff-service-protocol-v4:sha256
```

`scripts/registry-service-receipt.test.mjs`不在本任务允许写入范围，不能为制造组合入口绿色而越界
修改。最小successor只需显式扩展该单文件ownership，把五项receipt expectation迁到
v9/v10/v7/v5/v5，然后从clean scoped implementation重新运行canonical命令。此处没有把该
skiff-packages oracle mismatch误报为Skiff production blocker。

## 5. Reverse search

任务允许的production文件中，v7/v8/v6/v4旧generation精确为0命中。
`registry/pointer_store.skiff`对这些旧prefix和current generation prefix都为0命中，证明它不拥有
generation validator。

两份允许的test文件中，旧generation精确只有5处命中，全部位于具名测试
`immutable records reject previous package and contract generations`：

```text
PackageArtifact schema v7
package build v8
Local ABI v6
ServiceContract schema v4
service protocol v4
```

production validator和全部positive runtime fixture均命中current
v9/v10/v7/v5/v5/v2/v2。范围外positive receipt oracle的5项非current命中已在第4节逐行列出，
也是不能写PASS的唯一原因。

## 6. 静态检查与边界

| 检查 | 结果 |
| --- | --- |
| `npm run type-check` | PASS |
| `git diff --check` | PASS |
| changed-file boundary | 只有`registry/immutable_store.skiff`与两份允许的Registry test |
| 20-operation source oracle | 6/6 Node source suite通过；runtime publish列出20 available |

没有修改`registry/api.yml`、`registry/service.yml`、`package.yml`、model、20项operation、
`registry/pointer_store.skiff`、其它package、任何lockfile、Skiff production/test/fixture或
Internals。没有访问stable/live，没有merge、rebase或push，也没有派子Agent。
