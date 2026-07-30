# P5-F309 Capability-context platform catch consumer结果

状态：Completed。

任务提交：`f4fb936676819f97ca09e23c811c9032c0476a79`。

集成提交：`655b860f2f0d5a29226375f7f214a0f8204093a4`。

## 结果

- File、ProviderUnavailable、Protocol、Cancel与Timeout使用exact
  `PlatformBuiltinErrorIdentity`；
- decode、unsupported、resource-limit及ordinary diagnostics保持`None`；
- File Opaque/Stream/Execution、Capability Opaque、DB Opaque及Stream Producer继续精确转发inner
  projection；
- cancelled budget保持Cancel，其余deadline/instruction budget保持Timeout；
- test fixture不再创建`test.*` platform identity；
- payload、display及cancel/timeout选择未改变。

## 验证

- capability-context list/full：PASS，29/29；
- crate fmt与`git diff --check`：PASS；
- 旧`TypeIdentity`、string builtin与fake platform identity反搜：零。

R1关闭，解除native与service-db consumers。

