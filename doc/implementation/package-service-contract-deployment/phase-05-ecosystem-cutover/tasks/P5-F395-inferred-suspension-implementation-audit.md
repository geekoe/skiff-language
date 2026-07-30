# P5-F395 Inferred suspension implementation audit

状态：Ready（只读；按F391拆分schema/compiler/runtime实现DAG）。

## 直接父节点

- `P5-F391-inferred-suspension-contract-design-result.md`

F391已冻结完整语义与A–E字段类别。本节点不得重新讨论关键字、interface guarantee或ServiceContract
是否包含callee summary；只把production现状变成可执行、无重叠的实现DAG。

## 必须完成

1. 全量枚举F391 A–E每个production字段、copy/normalize/validate/identity/runtime consumer和direct
   fixture/golden；给出文件、类型、函数、字段及当前用途。
2. 精确区分：
   - 删除的interface/callback/service protocol字段；
   - 必须保留的concrete executable/Package callable summary；
   - 从ServiceContract迁到implementation/deployment metadata的runtime fact；
   - 仅caller-side由target种类推导的suspension。
3. 确定strict schema/current generation变化：
   - PackageArtifact/Local ABI/build；
   - PackageSchema callback descriptor；
   - ServiceContract/ServiceProtocol；
   - 必要时ServiceDeployment/RuntimeAssembly；
   - 每个prefix/golden及拒绝legacy字段的owner。
4. 追踪runtime ordinary/async lane、callback adapter、WebSocket contract plan、service call cancellation；
   证明去掉contract bit后如何从linked concrete implementation或统一boundary语义运行，不能只删除
   validator。
5. 给出最小无重叠DAG及每节点：
   - production write set；
   - dependency；
   - 正负测试；
   - focused命令；
   - identity/rebuild receipt；
   - scope expansion条件。
6. 全量扫描真实interface/concrete pair、callback schema和ServiceContract，给出迁移后预计受影响artifact
   及最小生态重建顺序；Relay必须成为首个真实2-operation proof。

## 边界与交付

Skiff/Internals/skiff-packages production只读。可生成temporary IR/artifact、跑现有聚焦测试，不修改源码，
不访问stable/live/外部服务，不派子Agent。

在本任务worktree写
`P5-F395-inferred-suspension-implementation-audit-result.md`。若F391语义足够执行，返回
`TASK_EXECUTABLE`和唯一DAG；只有发现与冻结规则真正矛盾时才返回精确用户决策题。

result本地commit、worktree clean；不merge/rebase/push。
