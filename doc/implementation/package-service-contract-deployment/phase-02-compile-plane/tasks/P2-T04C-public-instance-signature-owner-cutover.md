# P2-T04C：Public-instance Signature Owner Cutover

## 目标

把canonical PackageArtifact public-instance projection从旧publication-shaped `OperationAbiRef`中间owner切到
纯execution target，使File IR execution signature不再承担source conformance或semantic ABI职责。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”“Package-local ABI 与
Service ABI”及“Compiler 与 Projection 流水线”章节。

## 依赖与写域

- 依赖T03H、T04B；与T03I并行，组合正确性由两者共同验收。
- 独占`compiler/compiled`与`compiler/projection-input`中的public-instance semantic handoff，以及
  `compiler/projection/src/package_artifact`中的public-instance discovery/target internal model、callable target
  consumer与直接测试；必要时窄改既有compiled public-instance handoff test的断言。
- 不修改source、lowering、artifact-model/identity schema、runtime或integration fixtures。

## 完成态

1. compiled只从T03H validated interface query构造public-instance handoff；删除从File IR receiver/interface method
   signature或`TypeResolutionModel`重算`implements_interface`、canonical type args与method signatures的adapter。
2. projection-input只携带source-validated interface/method key与target discovery所需的typed结构事实；删除
   `implements_interface`布尔、`package_interface_methods`、`receiver_implements_package_interface`及等价的
   可漂移第二owner。
3. canonical PackageArtifact public-instance discovery只产生内部typed execution link，至少包含public path、
   receiver const/executable target与Local ABI建图所需的interface/receiver facts；不生成或保存
   `OperationAbiRef`、publication operation id或legacy method operation carrier。
4. 删除File IR executable/interface signature到`CanonicalPublicCallableSignature`的转换、相等比较和
   `public_instance_method_operation_abi_id`调用。projection只校验method/const/module/index/duplicate等结构事实；
   semantic conformance由T03H source owner完成。
5. public-instance callable exact signature仍且只从T04B
   `ProjectionPackageCallableSignatureFacts`按`(publicPath,module,index)`附着；contract/local/container/nullable、
   receiver trimming、missing/duplicate/extra/target mismatch保持fail closed。
6. terminal `PackageArtifact`仍只保存`PackageCallableId`、exact `PackageLocalAbi`与execution links；不把exact
   signature倒灌回File IR或legacy `CanonicalPublicCallableSignature`，不修改wire schema。
7. legacy DTO若仍被其它非canonical adapter引用，不在本任务增加compatibility；canonical
   `compiler/projection/src/package_artifact` production path中的旧public-instance operation identity owner归零。

## 聚焦验收

- `public_instance_signature_handoff`恢复PASS，并证明File IR opaque signature不影响exact Local ABI contract type。
- projection direct tests覆盖missing method/index、duplicate interface method、exact signature
  missing/extra/target mismatch及public-instance method map/target。
- 反向搜索canonical PackageArtifact public-instance production path中`OperationAbiRef`、
  `public_instance_method_operation_abi_id`和execution-signature conformance比较归零。
- 反向搜索compiled/projection-input canonical path中File IR/TypeResolutionModel conformance重算、上述布尔与
  interface-method DTO归零。
- 运行compiled/projection最小测试/check、changed-file rustfmt与`git diff --check`，不运行Phase gate。

## 执行合同

- DAG：波次9e，与T03I按文件ownership并行；两者共同解除R10I evidence refresh与production复验。风险：高；
  public-instance terminal projection checkpoint。
- worktree：`/Users/geek/workspace/skiff-p2-t04c-public-instance-owner`；分支：
  `codex/p2-t04c-public-instance-owner`；从含T03H/T04B/R10I的integration HEAD创建。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试或宽泛盘点。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；public-instance target model、exact signature
  handoff或PackageArtifact callable surface变化即失效。
