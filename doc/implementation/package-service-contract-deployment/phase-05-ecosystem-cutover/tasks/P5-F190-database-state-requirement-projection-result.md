# P5-F190：Database State Requirement 投影结果

状态：Completed

## 直接父任务

- `P5-F190-database-state-requirement-projection.md`

## 交付

- `package.yml`成为package runtime state requirement的canonical声明owner：
  - `state.<key>.kind`解析为typed `PackageStateRequirement`；
  - package声明不接受deployment-owned `namespace`；
  - `namespace`只由`config.<environment>.yml`的state binding提供。
- `PackageRuntimeRequirements`新增独立`state`集合。Database不再伪装成
  `resources`中的external resource，也不按package名、service名或固定key推断。
- compiler把package state声明与lowering产生的DB schema事实合并：
  - 有DB schema但无database requirement时fail closed；
  - 声明database requirement但无DB schema时fail closed；
  - 当前一个activation只有一个DB capability，因此多个database requirement key fail closed；
  - requirement按key规范化并进入PackageArtifact build identity。
- boundary projection把typed package state投影为`Database`、`Redis`、`Actor`或`Queue`；
  deployment projection要求selected callable事实与PackageArtifact声明精确一致，并聚合完整package
  closure的state requirements。
- deployment state binding按`requirementKey + kind`精确校验：
  缺失、多余、错误kind均fail closed；合法binding中的namespace原样进入ServiceDeployment，
  RuntimeAssembly activation template及Runtime `StateBinding`继续消费同一typed事实。
- PackageArtifact identity validator拒绝空key、重复key和多个database requirements。

## 真实 Registry 证据

使用真实`skiff-packages` Registry root和临时canonical artifact store完成bootstrap及
`package build registry`。生成的PackageArtifact包含：

```json
{
  "runtimeRequirements": {
    "config": [],
    "state": [
      {
        "key": "registry-store",
        "kind": "database"
      }
    ],
    "resources": [],
    "runtimeCapabilities": []
  }
}
```

同一次build生成的ServiceDeployment包含：

```json
{
  "stateBindings": [
    {
      "requirementKey": "registry-store",
      "kind": "database",
      "namespace": "skiff-run-registry"
    }
  ]
}
```

没有删除真实binding，也没有在Runtime按`registry-store`或package id猜数据库。

`SKIFF_ROOT=<F190 worktree> npm run test:registry`中的source检查和真实package build通过；
随后package test入口因既有CLI root detection把合法的`package.yml + service.yml`误判为
`contains both package.yml and service config`而停止。该独立blocker不在F190 state链内，已由
Registry任务报告给后续CLI修复。

## 验证

通过：

```text
cargo test --offline -p skiff-compiler-input \
  parses_typed_package_state_requirements_without_deployment_namespace
1 passed

cargo test --offline -p skiff-compiler-projection \
  package_artifact::runtime_requirements::tests
2 passed

cargo test --offline -p skiff-deployment
48 passed

cargo test --offline -p skiff-artifact-identity
87 passed; 1 ignored

cargo test --offline -p skiff-runtime-host loader::assembly_admission
26 passed

cargo test --offline -p skiff-runtime-loader runtime_assembly
10 passed

cargo test --offline -p skiff-compiler --test db_process_metadata
7 passed

cargo check --offline --workspace
passed

真实 Registry bootstrap + package build
passed

git diff --check
passed
```

完整compiler测试推进至integration基线已有的`prelude_std_schema`断面；
当前integration同一测试仍有`stream_type_is_explicitly_boundary_unavailable`失败。
F190新增的DB compile、state projection、deployment matrix与Runtime admission测试全部通过。
