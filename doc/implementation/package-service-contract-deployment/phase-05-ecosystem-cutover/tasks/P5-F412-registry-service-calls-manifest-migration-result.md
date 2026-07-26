# P5-F412 Registry serviceCalls manifest migration result

状态：Complete。

## 1. 提交锚点与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| skiff-packages start | `3653a294cfb92e60e220dcccc94bc8e8add65b33` | `93602c0e99ef15cf539334931e522d5ba844871c` |
| agent implementation | `cf21ba53b32889b45cc1902947a12714701e3806` | `1849f97a1f1217b95e6e349bc529eaaf220a62f4` |
| integration cherry-pick | `0972e65604cd4cfd45bcdb289cfe5019f57dc265` | `1849f97a1f1217b95e6e349bc529eaaf220a62f4` |
| Skiff toolchain | `8c719f7e4e5ac5233fc091b3f4da7d2a802586f7` | `8d6f033b4cce620a59ff5eac6c4257008d50e7bf` |

skiff-packages 改动严格为：

```text
registry/api.yml
registry/service.yml
scripts/registry-service-source.test.mjs
scripts/registry-service-receipt.test.mjs
```

没有修改 Registry 实现、storage tests、其它 package、共享 schema 或生态外仓库。

## 2. 迁移结果

- `api.yml` 的 20 个函数全部由旧 `source + serviceCall: true` object 改为 scalar selector；
- 类型公开面保持不变；
- `service.yml.serviceCalls` 精确列出 canonical 20 个 Registry operation，无重复；
- source test 同时拒绝旧 `source:` 与 `serviceCall:` leaf；
- fresh PackageArtifact 为 v8，package build 为 v9，Local ABI 保持 v6；
- Package record 不再存在 `serviceCallRoots`；
- ServiceContract protocol 为 v4，contract operation ID 为 v1；
- receipt、contract、deployment operation 数精确为 `20 / 20 / 20`；
- deployment binding 直接保存 receipt 中同一个 exact `PackageCallableId`；
- gateway、deployment ingress、assembly ingress 保持 `0 / 0 / 0`。

## 3. Fresh receipt 与验证

```text
Package build:
skiff-package-build-v9:sha256:7e76447e1766344e220d5c82b9a0351baff466b320acf012b9b85e558ffbd886

Package Local ABI:
skiff-package-local-abi-v6:sha256:c95b1889e2044c5780969ec040cfa9ec2c91afe9ba69a0c96574b733b4e71d73

Service protocol:
skiff-service-protocol-v4:sha256:0af6b732f66ace9cf285428624f9fbf4690f65382377d2c63f8ab6d259c98f32

Service deployment:
skiff-deployment-artifact-v2:sha256:b2374d06f564a0dc39413179c03b4a4ed0de0b2fa8c780936fabe1801a3dbe36

Runtime assembly:
skiff-runtime-assembly-v2:sha256:718fda78a54603ad919955c7282beda95b3554d96d4c4b3ad215b23d2911a2f7
```

执行结果：

- Registry source：`6 passed / 0 failed`；
- Registry fresh receipt：`1 passed / 0 failed`；
- combined authoring：`7 passed / 0 failed`；
- `npm run type-check`：PASS；
- `git diff --check`：PASS。

没有运行 F381 storage suite、stable/live 或外部服务。P5-F412 完成 Registry 的新 selection
authoring；storage current-generation 动态验证仍是独立后继节点。
