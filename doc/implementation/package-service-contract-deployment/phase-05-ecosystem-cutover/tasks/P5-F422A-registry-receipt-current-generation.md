# P5-F422A Registry receipt current-generation closure

状态：Ready（F422唯一范围外oracle后继）。

## 直接父节点

- `P5-F422-registry-current-generation-storage-closure-result.md`

F422的Registry production与9项真实runtime测试已通过；canonical `npm run test:registry`唯一失败是
`scripts/registry-service-receipt.test.mjs`仍冻结五个旧generation expectation。本节点只同步该
non-production receipt oracle并重跑canonical组合入口，不修改production或再次实现storage。

## 精确起点与唯一写入

skiff-packages：

```text
/Users/geek/workspace/skiff-packages-phase-05-integration
commit 1bc4504681e037fde4bfc92cd7b36f85a56b0fe0
tree   d39c43c9a4c3dabaa287c01c44521b8d156cff8e
```

唯一写入：

```text
scripts/registry-service-receipt.test.mjs
```

Skiff task/result base：

```text
/Users/geek/workspace/skiff-phase-05-integration
commit 9fc4fc5db7051bf751246fa126c38a7254d47b5b
tree   aa14118b5efed0efb3ade1b4ac141fd219c2855e
```

唯一写入为本任务result。启动时验证exact、clean；task文档提交只造成允许的文档增量。

禁止修改Registry production、`.skiff` tests、manifests、20项operation、其它script、Skiff
production/test或任何lockfile。不得派子Agent、merge/rebase/push/stable/live。

## 修改与验证

只把positive fresh receipt的五项expectation精确同步为：

```text
PackageArtifact schema      skiff-package-artifact-v9
Package build prefix        skiff-package-build-v10:sha256
Package Local ABI prefix    skiff-package-local-abi-v7:sha256
ServiceContract schema      skiff-service-contract-v5
ServiceProtocol prefix      skiff-service-protocol-v5:sha256
```

ContractOperationId继续v1，ServiceDeployment/RuntimeAssembly继续v2；20项operations/bindings、
gateway count及其它断言不得放宽。旧值反向搜索在该文件必须为0。

执行：

```bash
cd /Users/geek/workspace/skiff-packages-phase-05-integration
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration npm run test:registry
npm run type-check
git diff --check
```

必须记录Node source/receipt和runtime三类实际discovery/pass/fail/skip；runtime必须真实非零执行，
不得复用F422的9/9代替最终代码状态。

全部通过才在result写`REGISTRY_STORAGE_CURRENT_PASS`。skiff-packages implementation与Skiff
result分别单一commit，两个worktree clean；完成后不得自行承接新节点。
