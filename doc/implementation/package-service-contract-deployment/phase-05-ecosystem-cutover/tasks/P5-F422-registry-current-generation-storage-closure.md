# P5-F422 Registry current-generation storage closure

状态：Ready（F381 checkpoint在当前generation与当前test-runner上的收口）。

## 直接父节点

- `P5-F381-registry-current-generation-storage.md`
- `P5-F381-registry-current-generation-storage-blocker.md`
- `P5-F421B-suspension-relay-first-ecosystem-proof-result.md`

F381已完成四类immutable/pointer动态用例，但冻结在旧的
PackageArtifact v7/build v8/Local ABI v6、ServiceContract/protocol v4。其真实runtime test当时被
package-test assembly旧入口遮挡。当前Skiff N4已经通过，F421B fresh records进一步证明current
generation是v9/v10/v7/v5/v5/v2/v2，并且Registry本身可以由current CLI发布。本节点不改变Registry
API、20项service call或存储语义，只把既有checkpoint迁到current generation并执行被遮挡的真实测试。

## 精确起点

Skiff toolchain/result repo：

```text
/Users/geek/workspace/skiff-phase-05-integration
commit 1f289d8116f90448421566630798d54922c712eb
tree   203c01a72908356fec4cc2ead75efbfc1bf32b65
```

skiff-packages implementation worktree：

```text
/Users/geek/workspace/skiff-packages-p5-f422-registry-storage
branch codex/p5-f422-registry-storage
commit 77eac6611e790ed06a6c381379467c08a9391b0a
tree   c151e89ed9976975d9316945ee2f171b4ac590aa
```

`77eac661`是F381 implementation在current
`0972e65604cd4cfd45bcdb289cfe5019f57dc265` integration上的干净移植。启动时验证两个repo clean、
exact commit/tree；Skiff task checkout允许只追加本任务文件。

## 写入边界

skiff-packages允许：

```text
registry/immutable_store.skiff
tests/registry/immutable_store.test.skiff
tests/registry/pointer_store.test.skiff
```

只有测试发现既有pointer production也硬编码旧generation时，才可修改
`registry/pointer_store.skiff`，并须在result中给出精确命中；没有命中不得触碰。

Skiff允许：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
P5-F422-registry-current-generation-storage-closure-result.md
```

禁止修改`registry/api.yml`、`registry/service.yml`、`package.yml`、model、20项operation、
其它package、Skiff production/test/fixture、Internals或任何lockfile。Skiff尚未发布，不添加历史
兼容、双读或“任意prefix”分支。不得派子Agent、merge/rebase/push/stable/live。

## 唯一generation迁移

四类storage validator与全部正负fixture精确收敛为：

| record | schema | identity |
| --- | --- | --- |
| PackageArtifact | `skiff-package-artifact-v9` | build `skiff-package-build-v10:sha256`；Local ABI `skiff-package-local-abi-v7:sha256` |
| ServiceContract | `skiff-service-contract-v5` | protocol `skiff-service-protocol-v5:sha256` |
| ServiceDeployment | `skiff-service-deployment-v2` | `skiff-deployment-artifact-v2:sha256` |
| RuntimeAssembly | `skiff-runtime-assembly-v2` | `skiff-runtime-assembly-v2:sha256` |

旧v7/v8/v6/v4只允许出现在明确拒绝旧generation的负例中；production与positive fixture必须0命中。
不得把identity hash内容当作可验证的canonical preimage；storage层只验证该API当前负责的strict
schema/prefix、key/content/CAS语义。

## 动态验收

必须从clean checkpoint执行：

```bash
cd /Users/geek/workspace/skiff-packages-p5-f422-registry-storage
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration npm run test:registry
npm run type-check
git diff --check
```

并分别记录：

1. source/receipt Node tests的discovery、pass/fail/skip；
2. Registry service test的实际discovery与execution；不得把零case或compile-only当PASS；
3. 四类immutable各自put、相同内容replay、read；
4. 四类pointer各自初始CAS、第二次CAS、current read、ascending history；
5. content conflict、malformed identity、CAS mismatch、candidate/release mismatch、非法history limit；
6. 旧generation拒绝；
7. test-runner是否已越过F381记录的
   `package-test ingress is not yet migrated to deployment gateway entries`。

若current test-runner暴露新的Skiff production blocker，继续完成彼此独立的Node/type/diff检查后，
写`TASK_SCOPE_EXPANDED`并停止，不得修改Skiff。若Registry源码/测试本身存在任务范围内的机械问题，
直接修复并重跑聚焦命令。

## 交付

skiff-packages提交一个current-generation implementation/test commit；Skiff提交一个独立result
commit。两个worktree最终clean。result必须包含：

- exact start、implementation、result commit/tree；
- 三类测试实际计数；
- 四类immutable/pointer矩阵与负例；
- current/old prefix反向搜索；
- 未访问stable/live、未改变20项service API。

全部通过才写`REGISTRY_STORAGE_CURRENT_PASS`。完成后不得自行承接新DAG节点。
