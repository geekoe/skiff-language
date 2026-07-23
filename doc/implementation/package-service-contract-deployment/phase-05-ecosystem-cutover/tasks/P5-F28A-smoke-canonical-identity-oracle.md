# P5-F28A：Smoke Canonical Identity Oracle

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第9条、§3“Package 与 PackageArtifact”、§13
“Registry、Release 与 Publish”和§14“Fail-closed 条件”。PackageBuildId来自规范PackageArtifact内容；smoke consumer只
校验typed coordinate、identity framing及receipt内部引用关系，不建立第二个内容hash事实源。

DAG节点F28A，依赖D39 complete；完成后解除F28B与I28的identity分片。当前共享接口为F27A
`PublishedPackageArtifactReceipt`→F27B `CanonicalStdSeedReceipt`→bootstrap JSON。风险高，验收分组为R29 bootstrap
Rust→JS边界。精确production base为`8982107308c021fe9a72ad9446e1820395a0bc83`，后续仅有D39合同文档提交。

写入边界仅：

- `scripts/lib/package-service-ecosystem-smoke-oracle.mjs`；
- `scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs`；
- oracle的直接Node tests，以及为I28新增的单一actual bootstrap→JS oracle无服务交接test/helper。

要求：

- 删除oracle与fake fixture中的手写expected std build hash；不得机械换成`2541456b...`；
- 接受任意合法`skiff-package-build-v4:sha256:<64 lowercase hex>`且内部artifact/pointer/record path完全一致的std receipt；
- coordinate错误、identity framing错误、artifact/pointer不一致、record path不一致、missing/extra keys继续fail closed；
- actual bootstrap交接test必须消费真实`--bootstrap-only --locked` JSON再调用同一production oracle，不复制receipt或hash；
- 不新增`preludeIdentity`或公共receipt字段，不改compiler/test-runner/Router/runtime。

直接验证只运行专用Node tests/syntax/diff-check；实际Cargo bootstrap交接由I28唯一运行，开发Agent不得运行Cargo、真实smoke、
full/I16/Host/stable。一个clean commit；完成后成熟度仍为Implementation Checkpoint。smoke oracle/fixture/bootstrap schema变化
使证据失效。
