# P1-T00：冻结 Service Contract Surface 与版本边界

状态：`ready`
类型：架构前置任务
依赖：无
执行者：独立文档 Agent，一份提交

## 目标

在 artifact DTO 和 package service requirement 落地前，冻结“package 编译时依赖的 service
contract究竟是什么”，避免把 deployment revision、provider package/build identity或尚未决定的
operation选择方式写进Code Unit。

## 已授权的结论

Agent应把以下结论同步为canonical契约，不重新讨论id模型：

1. 一个service contract由 `(serviceId, exact serviceVersion)` 发布，surface是**具名operation
   map**；每个operation映射到一个完整 `BoundaryOperationContract`。一个surface可以包含多个
   operation，不把service限制成单函数。
2. service deployment显式选择 root PackageUnit中 `Available` 的boundary callable，并映射成
   service operation name。具体YAML拼写留给Phase 02，surface模型现在冻结。
3. `serviceProtocolIdentity`只hash canonical operation map及其boundary contracts；不包含root
   package id/build、deployment revision、route、config/state值或artifact路径。
4. `serviceVersion`是contract release version；deployment revision是实现/配置/路由revision。
   同一service id/version的所有可激活revision必须具有相同protocol identity，package build可在
   boundary surface不变时替换。
5. package依赖记录service id、精确version、protocol identity及实际引用operation的typed
   expectation；provider package id/build id不作为地址。编译时读取到的build/path只作为artifact
   完整性证据。
6. Runtime Assembly在revision选择后，对每个 `(serviceId, version)` requirement必须恰有一个
   provider；缺失、冲突或protocol mismatch均fail closed。多个service id可复用同一PackageUnit。
7. 本轮package只能依赖已经发布到可信artifact root的service contract；初次发布的循环service
   contract不支持并fail closed，不在Phase 01引入interface-first或placeholder contract。

## Artifact 边界

`ServiceProtocolContract`是typed contract value，可以作为当前/未来ServiceUnit的明确子对象；本
任务不要求新增一类独立publication artifact。compiler input resolver只消费该contract view，
不得把整个deployment revision当作Code Unit依赖。

## 范围

必须同步直接相关段落：

- `doc/architecture/package-code-and-service-deployment.md`
- `doc/architecture/compiler-publication-pipeline.md`
- `doc/architecture/runtime-compiler-shared-artifact-types.md`
- `doc/architecture/release-registry.md`

只修改contract surface、identity、publication order和deployment revision边界，不扩写registry
功能。

## 非目标

- 不决定service manifest的最终YAML字段名。
- 不实现Rust/JS代码。
- 不设计version range、兼容性协商、optional/dynamic discovery或remote transport。
- 不解决循环service的interface-first发布。

## 验收标准

- T03能唯一确定 `ServiceContractRequirement` schema和identity输入。
- T04能从artifact root提取contract view，而不依赖deployment build作为调用地址。
- Phase 02只需决定manifest拼写和projection接线，不再选择single operation还是surface。
- service version、protocol identity、deployment revision和package build identity四者无混用。
- provider唯一性规则在compiler、assembly和后续phase overview中一致。

## 聚焦验证

```bash
rg -n "ServiceProtocolContract|serviceProtocolIdentity|deployment revision|operation surface" \
  doc/architecture/package-code-and-service-deployment.md \
  doc/architecture/compiler-publication-pipeline.md \
  doc/architecture/runtime-compiler-shared-artifact-types.md \
  doc/architecture/release-registry.md
git diff --check
```

## 停止条件

若canonical文档要求同一service id/version同时允许多个不同protocol identity，或要求package按
provider package/build寻址，整理最小冲突后询问用户。

## 提交

提交信息建议：`docs: define service contract surface and revision boundary`
