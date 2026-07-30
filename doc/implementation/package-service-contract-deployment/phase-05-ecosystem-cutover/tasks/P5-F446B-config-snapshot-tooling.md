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
- secret source必须ignored；POSIX平台读取前要求普通非symlink文件且mode精确`0600`，否则fail closed；
- tooling任何必要明文复制/暂存写完后必须先chmod到`0600`并重新确认，才可读取、overlay或publish；
  snapshot store目录/文件保持`0700`/`0600`；
- 无POSIX mode平台必须明确使用可验证的等价owner-only ACL、普通文件及link/reparse substitution防护；
  没有等价实现时fail closed；
- stale snapshot不作为latest fallback；
- dev/watch/publish/activation client传递snapshot ref，不把值重新塞回deployment或assembly。

本任务不设计KMS wire。可以预留独立snapshot store接口，但不得实现字段SecretRef、value-level envelope或
与DB keyring耦合的临时加密。

## Evidence

覆盖base/profile/secret三层、deep map、sequence/scalar replace、tombstone、unknown Package、type mismatch、
missing required、optional missing、diamond、cross-deployment隔离、target environment写入与missing/
substitution负例、permissions、atomic failure和cold read。

secret source permission决策已经关闭；实现与验收必须使用上述strict规则，不得降级为warning、
best-effort chmod或“读取后再检查”。
