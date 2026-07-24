# P5-F174：Test Runner Package Schema Cutover Result

状态：Completed

## 直接父任务

- `P5-F174-test-runner-package-schema-cutover.md`

## 交付

- test-runner新增共享的Package schema closure helper，从operation参数、返回、typed error、stream、
  callback根和canonical record descriptor递归计算精确传递闭包。
- helper同时产出按Package owner分组的`PackageTypeRequirement`和与requirements完全一致的真实
  `PackageSchemaTypeRecord` map；缺record或owner/key错配fail closed。
- ecosystem smoke contract使用生产Package的resolved canonical records生成requirements，并把同一
  精确record closure传给deployment projection。
- package-test contract使用overlay编译产物的resolved canonical records执行相同流程，保持overlay、
  selector和deployment identity语义。
- 无命名边界类型自然产生合法空requirements/records；没有空值fallback或payload伪造。
- test-runner内已删除`boundary_schema`及旧contract-owned schema类型引用。

## 验证

通过：

```text
rg "boundary_schema|ContractSchemaType|ContractTypeId" test-runner
# no matches

git diff --check
```

`cargo test --locked -p skiff-test-runner`成功编译test-runner及全部测试目标，执行结果为25 passed、
2 ignored、4 failed。四个失败均位于既有`canonical_std_seed`测试，原因是编译器当前拒绝
`external named types cannot enter package schema v1`，不经过本任务修改的smoke/package-test
contract路径。

`package_service_contract_deployment`目标执行为9 passed、1 ignored、7 failed；失败来自同一既有
compiler限制、dependency schema resolver fixture，以及`package_service_host_fixture`仍传额外record，
不属于本任务限定的两个consumer。

```text
cargo check --locked --workspace
```

workspace check已越过test-runner，当前首错位于`runtime/host/src/loader/active_assembly_context.rs`
的`contracts`类型推断，属于后续合流断面。
