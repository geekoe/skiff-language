# P5-F144B：Registry 真实 Service 重验续作结果

结论：`TASK_NOT_EXECUTABLE`

## 父节点

- `P5-F149-authoring-reachable-package-closure-result.md`
- 原 result：`P5-F144-registry-real-service-revalidation-result.md`

## 新共享 blocker

std closure 已通过，Registry package artifact与contract生成后，deployment projection失败：

```text
runtimeAssemblyPointerRead boundary contract does not match operation descriptor
```

该 callable projection为 Available，effects/provenance均 analyzed。artifact 仍精确为 12 Available/8 既知 unavailable。
失败属于 package-public operation contract到generated contract descriptor canonicalization。

诊断 artifact：`/tmp/p5-f144b-artifacts.ugE2Rj`。

