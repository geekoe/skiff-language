# P5-F446B Config Snapshot Tooling

## Scope

实现service root的唯一配置构造链：

```text
config.yml
  <- config.<profile>.yml
  <- config.<profile>.secret.yml
  + exact ServiceDeployment/package closure
  -> RuntimeConfigSnapshot
```

- root直接是canonical Package ID mapping；拒绝unknown Package、`config/service/packages/secrets`旧包装和
  任何platform policy顶层shape；
- map递归合并、scalar/sequence替换、null tombstone；
- 按exact Package build的own typed requirements验证required/optional/type；
- snapshot内部按ServiceDeploymentRef隔离，diamond same-build只materialize一次；
- 使用随机opaque immutable ID，原子写入受信snapshot store；第一版允许明文，日志/receipt不输出值；
- secret source必须ignored；tooling复制/写入使用`0600`，目录`0700`，stale snapshot不作为latest fallback；
- dev/watch/publish/activation client传递snapshot ref，不把值重新塞回deployment或assembly。

本任务不设计KMS wire。可以预留独立snapshot store接口，但不得实现字段SecretRef、value-level envelope或
与DB keyring耦合的临时加密。

## Evidence

覆盖base/profile/secret三层、deep map、sequence/scalar replace、tombstone、unknown Package、type mismatch、
missing required、optional missing、diamond、cross-deployment隔离、permissions、atomic failure和cold read。
