# P5-D09：Host Test-Runtime DAG Cycle Audit

## 角色与结论

只读审计F04在R07后仍无法运行isolated production Host的原因，并核对F04→F05→F03C依赖是否成环。
不得编辑、提交、修复或给F04/R02 verdict。

结论为`DESIGN GO`：`skiff-runtime-host`的25个编译错误精确分为23个legacy package-test consumer断链与
2个activation direction-aware codec旧调用；其它error为零。F03C原先拥有这些修复，但F03C依赖R05、F05又依赖
F04 receive，而F04 receive要求真实Host最终结果，因此必须从F03C拆出F08/R08前置节点。

## 冻结设计

- 不适配旧`package-test.start`。旧frame只有package/build/entrypoint字符串，无法无歧义构造canonical
  `RuntimeAssembly`与`PackageTestEntrypoint { deployment, contract, operation }`；任何扫描、推导、兼容alias或
  host-local pointer都会建立第二个identity/admission owner。
- F08删除Host legacy package-test接收、cache、synthetic service与执行seam。`PackageTestRuntimeBuilder`只保留为
  test-owned canonical loader；F04唯一走assembly activation → `ActiveAssemblyRoute` →
  `RuntimeAssemblyRequestTarget` → production Host ingress → `InProcessBoundary`。
- 两个codec caller仅改用现有`encode/decode_assembly_activation_frame`并传精确方向，不改transport schema。
- F04先形成clean implementation checkpoint但不调用receive reviewer；root将其合入integration并唯一刷新shared
  lock。F08/R08后，F04原fixture必须观察`provider-observed-helper-mutated`，才能发起原六项窄接收。

F03C继续拥有cold startup/register、prepare/stage/commit/abort、nested request trust boundary、generation drain、
stream/typed WS pin、capability/actor/health与provisioning职责拆分；不再拥有legacy Host package-test seam与两个旧
codec调用。
