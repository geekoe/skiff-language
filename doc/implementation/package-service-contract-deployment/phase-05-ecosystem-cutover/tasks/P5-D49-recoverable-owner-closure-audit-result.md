# P5-D49：Recoverable Owner Closure Audit Result

结论：COMPLETE，规范已明确，无需用户设计选择。

assembly允许同`packageId`、不同`PackageBuildId`的合法code objects；recoverable LocalConcrete则使用更窄的
durable key `(owner=Package{package_id}, concrete_type_identity)`，明确禁止build/version/slot/artifact
identity。plain-data hook construction必须允许重复packageId；真正索引、编码或恢复package-owned
LocalConcrete时，当前linked program中0或多个packageId candidate必须fail closed。

唯一修复owner为`runtime/eval/src/recoverable_behavior.rs`：删除`new_for_execution`的assembly-wide
`unique_package_ids()` eager检查及死helper/import，保留`local_concrete_owner`按需歧义检查与method-table
conflict checks。禁止改变model/wire durable key、first-win、按build选择或compat/dual key。
