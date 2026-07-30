# P5-F377 Registry service-call authoring result

状态：Complete。

## Checkpoints

- skiff-packages production commit：
  `de66dc875183d9ccaeddf1fc412910a31a7aac2c`
- production tree：
  `93602c0e99ef15cf539334931e522d5ba844871c`
- 已合入skiff-packages integration：
  `3653a294cfb92e60e220dcccc94bc8e8add65b33`
- 验证Skiff toolchain：
  `087ada637bd845a603734826fa4eb80c48138a56` /
  `330e09e244830b4bb8ed7bd2543b1fb013d08749`

## 结果

`registry/api.yml`的20个function leaf现在全部显式声明`source + serviceCall: true`。未改函数名、签名、
实现、错误或external ingress。

fresh真实authoring计数：

| surface | count |
| --- | ---: |
| PackageArtifact service-call roots | 20 |
| API functions with unique `serviceOperationId` | 20 |
| ServiceContract operations | 20 |
| ServiceDeployment operation bindings | 20 |
| gateway entries / deployment ingress / assembly gateway ingress | 0 / 0 / 0 |

operation ID、contract operation key、deployment binding和package callable ID逐项引用闭合。Local ABI仍为
`skiff-package-local-abi-v6:sha256:c95b1889e2044c5780969ec040cfa9ec2c91afe9ba69a0c96574b733b4e71d73`，
说明只改变service-call选择面。

fresh identity：

- package build：
  `skiff-package-build-v8:sha256:5bfb71aec59d4ed643fb2ae49d633faaa2fb40aea1faec06c736352be889e754`
- ServiceContract：
  `skiff-service-protocol-v4:sha256:0af6b732f66ace9cf285428624f9fbf4690f65382377d2c63f8ab6d259c98f32`
- ServiceDeployment：
  `skiff-deployment-artifact-v2:sha256:723a37ad69f8a025baaaa0c76f4592c2966ac6f922a00f14d73cce3039cffa37`
- RuntimeAssembly：
  `skiff-runtime-assembly-v2:sha256:07f358ea3eca658dbb78f0701dfc4ad4701fc87a5a259e6b0c8b39642d6d35f9`

测试：

- Registry source `5/5`；
- Registry真实receipt `1/1`；
- combined authoring `6/6`；
- skiff-packages type-check、reverse search与diff check通过。

`immutable_store.skiff`、`pointer_store.skiff`及其runtime tests保持不变，由F381迁移当前generation与四类
pointer成功路径。
