# P5-F377 Registry service-call authoring

状态：Ready。

## 直接父节点

- `P5-F375-registry-generation-revalidation-result.md`

父节点已用fresh artifact证明Registry的20个API目前都是package-only，真实ServiceContract为0。冻结目标仍是
20个ordinary service-call operation；本节点只修复authoring与receipt覆盖，不处理Registry存储generation
或Router启动问题。

## Checkpoint与worktree

- skiff-packages integration：
  `0ab4e7628b0a6aa90961c1485d2e58634b902676` /
  `5abb824e560778fd38a0a9a4e9936d189cc9f843`
- worktree：`/Users/geek/workspace/skiff-packages-p5-f377-registry-service-call-authoring`
- branch：`codex/p5-f377-registry-service-call-authoring`
- Skiff toolchain：使用包含本任务的
  `/Users/geek/workspace/skiff-phase-05-integration`，记录实际commit/tree。

## 必须完成

1. 在`registry/api.yml`为四类各五个、共20个function leaf显式声明`serviceCall: true`：
   - `packageArtifact{Put,Read,PointerRead,PointerCas,PointerHistory}`；
   - `serviceContract{Put,Read,PointerRead,PointerCas,PointerHistory}`；
   - `serviceDeployment{Put,Read,PointerRead,PointerCas,PointerHistory}`；
   - `runtimeAssembly{Put,Read,PointerRead,PointerCas,PointerHistory}`。
2. 不改变source binding、函数名/签名、错误或实现；不新增external gateway ingress。
3. 更新Registry局部source/receipt测试，使测试读取真实generated receipt/record并验证：
   - `PackageArtifact.serviceCallRoots == 20`；
   - JSON receipt 20个function全部有`serviceOperationId`；
   - `ServiceContract.operations == 20`且全部Available；
   - `ServiceDeployment.operationBindings == 20`；
   - gateway entries、deployment ingress和assembly gateway ingress均为0；
   - operation identity/reference闭合。
4. fresh bootstrap canonical std并真实publish Registry，再build-only生成assembly；不得读取stable store。

## 写入边界

允许：

- `registry/api.yml`；
- Registry局部authoring/receipt/source测试，以及确有必要的同目录小型fixture。

禁止：

- `registry/immutable_store.skiff`、`registry/pointer_store.skiff`及其runtime test；
- identity generation规则、artifact model、其它official package；
- Skiff、Internals、stable/live。

若20个显式marker仍不能生成20个operation，或需要改compiler/Router，返回`TASK_SCOPE_EXPANDED`。

## 验收与交付

先枚举并运行非零Registry source/receipt测试。真实receipt至少记录package build/Local ABI、
ServiceContract、deployment与assembly identity，以及20/20/20/0计数。运行`git diff --check`。

结果写入Skiff任务树由主Agent负责；本worktree production/tests一个本地commit、clean，不
merge/rebase/push。返回exact commit/tree、changed files、测试计数与fresh receipt。新Agent执行，不派
子Agent。
