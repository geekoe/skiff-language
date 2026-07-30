# P5-F446B Config Snapshot Tooling

## Scope

实现service root的唯一配置构造链：

```text
config.yml
  <- config.<profile>.yml
  <- config.<profile>.secret.yml
  + exact ServiceDeployment/package closure
  + trusted target environment
  -> RuntimeConfigSnapshot
```

- root直接是canonical Package ID mapping；拒绝unknown Package、`config/service/packages/secrets`旧包装和
  任何platform policy顶层shape；
- map递归合并、scalar/sequence替换、null tombstone；
- 按exact Package build的own typed requirements验证required/optional/type；
- snapshot内部按ServiceDeploymentRef隔离，diamond same-build只materialize一次；
- snapshot顶层required `targetEnvironment`只来自调用方提供的受信operator/activation坐标；不能从YAML、
  service ID、profile名、路径或ambient environment推断；
- 使用随机opaque immutable ID，原子写入受信snapshot store；第一版允许明文，日志/receipt不输出值；
- secret source必须ignored；tooling复制/写入使用`0600`，目录`0700`，stale snapshot不作为latest fallback；
- dev/watch/publish/activation client传递snapshot ref，不把值重新塞回deployment或assembly。

本任务不设计KMS wire。可以预留独立snapshot store接口，但不得实现字段SecretRef、value-level envelope或
与DB keyring耦合的临时加密。

## Evidence

覆盖base/profile/secret三层、deep map、sequence/scalar replace、tombstone、unknown Package、type mismatch、
missing required、optional missing、diamond、cross-deployment隔离、target environment写入与missing/
substitution负例、permissions、atomic failure和cold read。

`config.<profile>.secret.yml` source必须ignored且`0600`这一要求仍是当前authority。是否调整source文件
permission policy是尚未决定的独立问题；在用户明确修改前，本任务不得弱化或删除现有要求。
