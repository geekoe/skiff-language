# P5-F403 Service-calls manifest implementation audit

状态：Ready。

## 直接父节点

- `P5-F402-service-calls-manifest-selection-design-result.md`

父节点直接引用并更新Phase 05唯一权威设计。F400/F400A已被F402显式废止，不得作为实现依据。

## DAG位置与目的

- DAG节点：`service.yml.serviceCalls`实现前的唯一production/dataflow审计。
- 当前候选：Skiff Phase 05 integration commit
  `e17b543ef4bcaf936baafe7f9ed311c10d9a5fe7`。
- 完成后解除：shared artifact/parser checkpoint与compiler/service consumer迁移。
- 风险：高；source syntax、PackageArtifact wire/identity、ServiceContract projection与全部service fixture。
- 本节点只读，不修改production/test；只提交result文档。

## 必须回答

从真实production入口追踪并列出精确owner、符号与写入集合：

1. `api.yml`当前`serviceCall: true` parser/DTO/source resolution/projection的全部producer与consumer。
2. `PackageArtifact.serviceCallRoots`（含wire字段、identity、validation、loader/linker、schema generation、
   tests/checkers）的全部owner；确认删除字段需要的generation变化及所有reader同步点。
3. `service.yml` strict DTO/parser/validation如何增加`serviceCalls: string[]`，以及typed selection在何处
   解析到Package public function/public-instance root。
4. ServiceContract与ServiceDeployment当前如何从PackageArtifact roots生成operation/binding；给出改为
   `PackageArtifact public graph + typed service selection`后的单一数据流，禁止在deployment重新按字符串猜
   callable。
5. Package build identity、PackageLocalAbi identity、ServiceProtocolIdentity与deployment revision的当前
   preimage owner；列出证明F402 identity不变量的正反例。
6. 所有production/fixture/ecosystem `api.yml serviceCall`残留及对应`service.yml`迁移矩阵；区分普通
   function、public instance、zero-operation service与历史任务/result文本。
7. package+service root canonical source路径；确认不得恢复service-only compiler owner，并列出需要迁移
   的current source而不是把它当第二种root。
8. 最小可执行DAG：先共享schema/model checkpoint，再列互不重叠consumer，标出每节点前置、写入owner、
   最早风险探针、验证命令与证据失效边界。

## 审计边界

必须覆盖：

```text
artifact-model/**
artifact-identity/**
compiler/input/**
compiler/source/**
compiler/projection*/**
compiler/contract/**
compiler/driver/**
deployment/**
runtime/loader/**
runtime/linker/**
test-runner/**
scripts/**
router/**（仅实际读取PackageArtifact/ServiceContract shape的部分）
```

并对`/Users/geek/workspace/internals`、
`/Users/geek/workspace/internals-phase-05-integration`、
`/Users/geek/workspace/skiff-packages`与
`/Users/geek/workspace/skiff-packages-phase-05-integration`做只读source/fixture inventory。

不要泛化review其它Phase 05问题，不运行完整workspace/live/stable gate，不访问外部服务，不派子Agent。
允许运行只读搜索、schema/identity聚焦测试列举与便宜parser tests来验证owner；每条命令需说明实际选择的
测试数。

## 交付

写`P5-F403-service-calls-manifest-implementation-audit-result.md`并本地提交。结果必须给出：

- `TASK_EXECUTABLE`或`TASK_SCOPE_EXPANDED`；
- exact candidate commit/tree；
- producer→artifact→contract→deployment的代码路径；
- generation/identity结论；
- 精确迁移矩阵；
- 唯一DAG与建议任务文件边界；
- 未决问题是否需要用户决定。

worktree保持clean；不merge/rebase/push，不操作stable/live。
