# P5-F313 Linked-type-plan index fixture结果

状态：Completed。

任务提交：`0b5596b865d49fd4a82fd5faeca3307fdbcc4cbf`。

集成提交：`df148828cf38b730dbb64d839b8df8c877593709`。

- 唯一修改`runtime/linked-type-plan/src/assembly_seam.rs`；
- test image显式接收empty `ServiceErrorTypeIndex`；
- 旧3参数调用反搜为零；
- linked-type-plan list/full：PASS，16/16；
- `git diff --check`：PASS。

F310/R2关闭。

