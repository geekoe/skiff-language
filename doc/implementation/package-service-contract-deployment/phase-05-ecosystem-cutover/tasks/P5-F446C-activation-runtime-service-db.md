# P5-F446C Activation, Runtime Config And Service DB Cutover

## Scope

- Router activation prepare/commit/abort、durable state、control wire、health、drain与cold recovery同时携带
  exact RuntimeAssemblyRef和RuntimeConfigSnapshotRef；
- Runtime在prepare时先验证完整assembly/snapshot closure，再为每个
  `(ServiceDeploymentRef, exact Package build)`建立只读ConfigView；
- request、service call、continuation、stream、callback、actor和spawn沿ActivationContext传播相同snapshot
  owner，不读取ambient/latest/source YAML；
- service DB name/identity只由trusted `(platform, environment, serviceId)`派生；service/package authoring
  无输入；
- physical collection name由stable`(packageId, declared collection identity)`系统编码，不含build/version/
  alias；不同Package相同裸collection名保持隔离，无author mapping；
- 仅activation闭包含DB metadata时创建handle；同service所有Package共享handle，但DB target保持
  PackageArtifact/File IR/type identity；
- 跨service DB拒绝；test-only foreign target仍使用caller generated test service DB。

Router的`serviceDb.mongoUrl`继续只是operator-owned transport配置。service version、package version、
deployment revision、runtime replica和snapshot ID都不得改变数据库identity。

## Evidence

必须覆盖config-only generation、code-only generation、两ref同时变化、missing/tampered/cross-environment
snapshot、failed prepare保留active、cold recovery exact pair、same-build cross-service ConfigView隔离、
DB-on-demand、upgrade/rollback同DB/collection、same-bare-name cross-Package隔离、cross-service拒绝和
foreign-test caller DB。
