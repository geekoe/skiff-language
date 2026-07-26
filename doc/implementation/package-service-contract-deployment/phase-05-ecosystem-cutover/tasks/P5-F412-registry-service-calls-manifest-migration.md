# P5-F412 Registry serviceCalls manifest migration

状态：Ready。

## 直接父节点

- `P5-F403-service-calls-manifest-implementation-audit-result.md`
- `P5-F409-typed-service-selection-contract-driver-result.md`
- `P5-F377-registry-service-call-authoring-result.md`

F403 沿 F402 权威设计冻结了新模型；F409 已实现 typed selection。F377 证明 Registry 的目标服务调用面
精确为 20 个公开函数。本节点只把 skiff-packages 的 Registry authoring 和相应 receipt oracle 从旧
`api.yml serviceCall` 标记迁到 `service.yml.serviceCalls`。

## 精确代码状态与写入范围

- skiff-packages start：
  `3653a294cfb92e60e220dcccc94bc8e8add65b33`。
- Skiff toolchain：
  `/Users/geek/workspace/skiff-phase-05-integration`，执行时记录 exact commit/tree。
- 只允许修改：

```text
registry/api.yml
registry/service.yml
scripts/registry-service-source.test.mjs
scripts/registry-service-receipt.test.mjs
```

不得修改 Registry `.skiff` 实现、storage tests、其它 package、Skiff/Internals、共享 schema 或设计。
不得移植 F381 storage checkpoint；它是后继节点。

## 必须实现

1. `registry/api.yml` 的 20 个函数 leaf 改为 scalar source selector；所有类型 exports 保持不变。
2. `registry/service.yml` 增加 `serviceCalls`，精确按现有
   `scripts/registry-service-operations.mjs` 的 20 个 public path 选择，不能遗漏、重复或增加。
3. source test 应验证：
   - API 恰好 20 个 scalar function selectors；
   - 不存在 `source:` / `serviceCall:` 旧 leaf；
   - manifest selection 与 canonical operation list 精确相等；
   - package/service/config/error 等既有断言不弱化。
4. fresh receipt test 应验证：
   - PackageArtifact schema v8、build identity v9、Local ABI v6；
   - Package record 不存在 `serviceCallRoots`；
   - ServiceContract protocol v4，receipt/contract/deployment 精确 20 个 operation；
   - contract operation ID 为 v1，deployment 直接绑定 exact `PackageCallableId`；
   - gateway/ingress 仍为 0；
   - 不从 PackageArtifact 重建 selection。
5. 不保留旧 marker 或旧 artifact field 的兼容断言。

## 验证与交付

运行并记录非零计数：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
  npm run test:registry-authoring
npm run type-check
git diff --check
```

receipt 必须使用 fresh temporary artifact root，不得读取 stable artifact store。不得运行 runtime
storage suite、stable/live 或外部服务；不得派子 Agent。

生产与测试改动提交为一个 clean commit；返回 exact base、commit/tree、changed files、source/receipt
测试数与 receipt 摘要。result 由主 Agent 写入 Skiff integration；不 merge/rebase/push。
