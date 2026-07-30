# P5-F178A：Actor Declaration Artifact Checkpoint

状态：Ready

## 直接父任务

- `P5-F177-builtin-artifact-and-native-type-removal-result.md`

## 范围

修改syntax、compiler input-model/compiled公共声明事实、artifact-model actor metadata及直接测试。
不得实现source type checking、lowering、std native函数或Runtime执行。

## 必须实现

- 解析显式声明：

  ```skiff
  actor DocHub id DocId {
    nextSeq: number
    pendingOps: Array<Op>
  }
  ```

- actor声明拥有name、精确id type和字段；不能generic，不能同时作为普通type/representation声明。
- shared compiled/artifact facts显式标记actor kind、id type和字段，不再通过
  `implements std.actor.Actor<Id>`猜测。
- actor ABI输入覆盖id type、字段布局/编码、公开成员方法签名和actor runtime ABI版本；本任务只建立
  typed DTO与canonical identity入口，不实现最终方法依赖图。
- 严格wire拒绝旧`ActorRef`、`native type`及用普通type implements Actor伪造actor声明。
- bootstrap不是普通Actor值；公共事实提供actor field shape供后续compiler在registry intrinsic参数
  位置校验。

## 验证

- syntax、input-model、compiled、artifact-model/identity聚焦测试；
- parse/roundtrip/duplicate/id缺失/legacy implements Actor负例；
- `cargo check --workspace`首错进入后续compiler actor consumer；
- `git diff --check`；
- 独立提交并写result。
