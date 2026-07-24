# P5-F166：Runtime Package Schema Hydration

状态：Ready

## 直接父任务

- `P5-F165-package-schema-deployment-ingress-result.md`

## 同步前置结果

- consumer import已由`P5-F164-package-schema-consumer-import-result.md`闭合。

## 范围

修改runtime loader/admission、filesystem resolver和其fixtures，定义runtime只读resolved schema closure。
不得修改runtime boundary/value plan、eval执行、native callback或consumer service。

## 必须实现

- `RuntimeAssemblyContentResolver`能按ServiceContract的`PackageTypeRequirement`读取精确
  `PackageSchemaTypeRecord`；filesystem实现复用F160 canonical store。
- loader/admission在激活assembly前一次性完成：
  - contract identity；
  - type record path/hash/owner/key；
  - required集合与records集合完全相等；
  - operation roots到descriptor传递闭包完全相等；
  - public-only index证据；
  - 无递归环。
- 形成不可变`ResolvedServiceSchema`（命名可调整），与admitted ServiceContract绑定并供后续boundary/eval读取。
- eval不得持有filesystem/store resolver，也不得按active PackageArtifact、version或provider source补类型。
- 同一PackageSchemaTypeId跨多个contract/assembly可复用record payload，但每个contract的required closure独立验证。
- loader接口及所有test resolver实现同步更新；不恢复`boundary_schema`或空fallback。

## 验证

- runtime loader聚焦测试覆盖真实filesystem records、共享record去重、缺失、额外、错hash/owner/key、
  contract未require、closure缺child、非public index及SCC。
- admission后删除/替换filesystem record不影响已固定的in-memory closure；新admission仍fail closed。
- loader crate恢复编译并通过聚焦测试。
- `git diff --check`；独立提交并写result，记录boundary/eval下一断面。

