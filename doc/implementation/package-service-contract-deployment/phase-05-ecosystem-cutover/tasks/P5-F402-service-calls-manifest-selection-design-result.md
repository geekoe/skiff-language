# P5-F402 Service-call manifest selection design result

状态：Complete。

## 决策

`api.yml`只拥有Package public graph：

```yaml
getUser: users.getUser

relayProxy:
  const: relay.relayProxy
  interfaces:
    - relay.CodexRelayProxyClient
```

`const`与`interfaces`只描述public instance：前者选择top-level singleton receiver，后者显式冻结
调用方可见的interface method surface。它们不选择service API。

`service.yml`使用单一数组选择ServiceContract roots：

```yaml
serviceCalls:
  - getUser
  - relayProxy
```

每个元素是`api.yml`完整public path，不是source selector：

- public function生成一个operation；
- public instance root展开其显式listed interfaces的全部methods；
- 不支持单独选择instance method；需要窄surface时定义窄interface或wrapper function；
- dotted public path用字符串表示；
- missing/empty数组生成零operation contract；
- duplicate、unknown、non-callable或boundary-unavailable path fail closed；
- 数组顺序不参与ServiceProtocolIdentity。

HTTP/WebSocket external ingress仍由`service.yml`独立拥有。一个source function可同时通过
`serviceCalls`成为service operation，并被HTTP entry引用为external handler；两者生成不同identity，
不能相互推断。

## Artifact与identity边界

PackageArtifact发布完整Package public callable graph、Local ABI、callable links与每个callable的
boundary projection。它不得读取`service.yml`，也不得保存`serviceCallRoots`或其它service selection。

ServiceContract projection消费：

```text
PackageArtifact public graph
+ typed service.yml.serviceCalls selection
-> operations + PackageSchema closure
```

只改变`serviceCalls`且Package source、`package.yml`、`api.yml`不变时：

- PackageArtifact identity bit-identical；
- PackageLocalAbi identity bit-identical；
- operation集合变化时ServiceProtocolIdentity变化；
- ServiceDeployment operation bindings与deployment revision相应变化。

ServiceDeployment不得要求作者再次维护operation-to-source映射；tooling从被选择public root的exact
PackageCallableId确定性生成binding。

## Source-root边界

本决策不改变权威架构中的source ownership：

- ordinary package：`.skiff + package.yml + api.yml`；
- service：在同一Package root上增加`service.yml`与可选`config.*.yml`；
- `package.yml`继续拥有Package id/version与package/service dependencies；
- `service.yml`拥有service id、`serviceCalls`和external ingress。

因此P5-F400把current service-only Relay source当成canonical compiler input，并要求移植
service-only owner的结论不再可执行。正确方向是让生态service source迁移到上述Package+Service root，
不是让compiler恢复第二种source owner。

## 文档变更

已统一：

- `doc/overview.md`
- `doc/reference/api-yml.md`
- `doc/reference/static-semantics.md`
- `doc/architecture/package-service-contract-deployment.md`
- `doc/architecture/compiler-package-pipeline.md`
- `doc/architecture/runtime-compiler-shared-artifact-types.md`
- `doc/architecture/compiler-entity-and-identity.md`
- `doc/architecture/gateway-runtime-adapter-boundary.md`
- `doc/architecture/release-registry.md`
- `doc/architecture/any-interface-value.md`
- `doc/architecture/recoverable-value.md`

后续实现必须删除`api.yml serviceCall` parser/model与PackageArtifact service-call roots，增加typed
`service.yml.serviceCalls`选择，并迁移所有service fixtures/source。Skiff尚未发布，不保留旧写法兼容。
