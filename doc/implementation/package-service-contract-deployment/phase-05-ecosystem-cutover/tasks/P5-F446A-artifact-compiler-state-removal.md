# P5-F446A Artifact And Compiler State Removal Checkpoint

## Scope

建立后续任务唯一共享checkpoint：

- 从ServiceDeploymentInput、ServiceDeployment、DeploymentArtifact、RuntimeAssembly、activation template及
  identity projection删除config literal、SecretRef、state/resource/runtime-capability/policy字段，删除
  `DeploymentPolicy`、`ResourcePolicy`、`ResourceBinding`与`RuntimeCapabilityBinding`类型；service级
  timeout、quota、principal、CPU和memory占位值不保留；
- 删除SecretRef DTO/validator/identity、PackageRuntimeRequirements.state、StateBinding、
  StateBindingKind、`PackageResourceRequirement`、`PackageRuntimeCapabilityRequirement`及所有public
  re-export；`PackageRuntimeRequirements`只保存本Package的typed config requirements；
- `package.yml state`成为unknown key并fail closed；
- Package compiler只从本Package源码收集own typed config requirements和DB metadata，不复制dependency；
- 定义strict `RuntimeConfigSnapshotRef`、snapshot DTO与committed generation中并列的assembly/snapshot refs；
  snapshot ID为随机opaque coordinate，不做content hash identity。

不得实现YAML overlay、filesystem store、Runtime ConfigView或DB连接。所有wire/schema代际一次hard cut，
producer/reader/golden/checker同commit更新，不兼容旧shape。

## Evidence

- identity mutation证明配置值不再影响deployment/assembly；
- strict-wire负例拒绝旧字段、missing snapshot ref、cross-field substitution；
- Router operator `requestTimeoutMs`仍是external business request唯一截止时间来源，service profile、
  deployment和assembly不能覆盖；
- compiler fixture证明service package own requirements、dependency requirements不复制、DB metadata不再要求
  manifest state；
- 反向搜索覆盖生产、public API、golden和checker，不只删主DTO。

删除边界必须精确：`package.yml.resources`、`PackageArtifact.staticResources`、resource blob
store/loader和`std.resource.*`属于Package静态资源能力，继续保留；`NativeRequiredContext`、
`NativeCapabilityContexts`及Router `runtime.capabilities` transport feature flags也继续保留，不能因名称
相似而当作上述空binding脚手架删除。
