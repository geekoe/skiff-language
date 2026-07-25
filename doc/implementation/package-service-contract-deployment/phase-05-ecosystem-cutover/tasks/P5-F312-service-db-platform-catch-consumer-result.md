# P5-F312 Service DB platform catch consumer结果

状态：Completed with unrelated suite blockers。

任务提交：`24e516b40ec19393aebe1ce64e163ed757a4c52f`。

集成提交：`bff402f093a3b3ea5a7bd5d84b286cd700d90c37`。

## 结果

- Mongo conflict使用exact `DbConflict.catch_identity()`并保留原details；
- DbDecode即使payload code为`std.db.DecodeError`仍为`None`；
- 其它ordinary error为`None`，Opaque精确转发inner projection；
- service-db旧`TypeIdentity`反搜为零。

## 验证

- list：PASS，102项；
- 聚焦catch tests：PASS，8/8；
- 排除两个已定位非本任务失败后：PASS，100/100；
- fmt与`git diff --check`：PASS。

完整套件2个既有失败：

- provider fixture使用含`/`的service id，被Mongo database-name校验拒绝；
- live Mongo用例因`127.0.0.1:27017`未运行失败。

未操作stable/live。R4 production关闭；上述环境/fixture问题不作为本迁移语义blocker。

