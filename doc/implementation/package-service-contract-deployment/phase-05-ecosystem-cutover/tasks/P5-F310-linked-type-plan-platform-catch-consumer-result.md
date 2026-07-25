# P5-F310 Linked-type-plan platform catch consumer结果

状态：Production implemented；标准测试被F313 fixture drift遮挡。

任务提交：`b6b37346d64e08e6025d84a7c489a52531c891c7`。

集成提交：`58771f640f7373e5f8461f19c6eb8febcfcd150f`。

## 结果

- protocol error使用exact `ServiceProtocol.catch_identity()`；
- boundary error继续转发inner projection；
- invalid artifact与ordinary diagnostic保持`None`；
- 删除旧`TypeIdentity`与string builtin，不改变payload/display。

## 证据

- linked-type-plan production `cargo check --lib`：PASS；
- 独立临时行为harness：2/2 PASS；
- crate fmt与`git diff --check`：PASS；
- 标准lib tests在枚举前被`assembly_seam.rs`的旧3参数
  `AssemblyExecutionImage::try_new`遮挡。F298已将第4参数改为required
  `Arc<ServiceErrorTypeIndex>`。

F313关闭该机械fixture后重跑标准list/full，才能关闭R2。

