# P5-D49：Recoverable Owner Closure Audit Fragments

冻结production commit `ad847f7254521d1dd4679a4f8af72b2c88753310`。三个只读分片一致确认：

- I02 fixture中的production与test overlay是同`packageId`、不同`PackageBuildId`的合法code objects；
  resolver/execution image按build去重，无producer重复或slot错位。
- 直接失败点是`EvalRecoverableBehaviorHooks::new_for_execution`无条件运行assembly-wide
  `unique_package_ids`；即使plain-data spawn未产生LocalConcrete值也被提前拒绝。
- 现有相关recoverable测试均走legacy service-owned fixture，未覆盖canonical重复packageId assembly；
  最小非E2E探针属于`skiff-runtime-eval`。

待汇总的契约分歧：

- D49A认为canonical concrete-code owner必须是`PackageBuildId`，durable owner/model与restore key也应升级。
- D49B认为recoverable durable规范明确以`packageId`为LocalConcrete owner且禁止build/version/slot；应只移除
  eager全局检查，真正生成LocalConcrete key时继续按需歧义拒绝。
- D49C确认当前model只有`packageId`，若允许歧义LocalConcrete则表达不足；测试不能代替契约裁定。

汇总owner必须对照权威设计和recoverable规范，区分plain-data hook construction、LocalConcrete encode与restore；
若规范已明确则选唯一最小production owner及正负探针，若冲突则报告最小用户设计问题。禁止编辑或运行gate。
