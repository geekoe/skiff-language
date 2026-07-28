# P5-F445H-I7-L Service-scoped assembly consumer

状态：`IMPLEMENTED_PENDING_R_WIRE_JOIN`。

## 1. Parent and baseline

直接父节点：

- `P5-F445H-I7-K-service-scoped-ingress-canonical-result.md`；
- `P5-F445H-I7-D0-service-scoped-ingress-design-result.md`。

唯一架构事实源是
`doc/architecture/package-service-contract-deployment.md`。本节点消费K已经冻结的
`ServiceIngressKey = (ServiceDeploymentRef, IngressSelector)`与v4/v3/v2代际，不修改
canonical schema、identity或Router生产方。

| 项 | 值 |
| --- | --- |
| baseline commit | `1a11328a241b5d177eb40885e294fe31d65a7240` |
| baseline tree | `ca1f7c2f040458df4275d00801eb0fc61046a1a8` |
| branch | `codex/p5-f445h-i7-l-assembly-ingress` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-l-assembly-ingress` |
| integration owner | `/root/phase05_integration_steward` |

## 2. Ownership

写集限于：

```text
deployment/src/assembly/**
runtime/loader/**
runtime/linker/**
runtime/host/**
runtime/request/**
```

以及这些owner直接使用的测试、fixture和本任务/result文档。禁止修改compiler/authoring、
Router TypeScript、artifact-model或artifact-identity canonical owner。

## 3. Required behavior

1. assembly resolver按`ServiceIngressKey`索引：
   - 不同service deployment共享相同protocol/method/path合法；
   - 同一精确deployment内重复selector失败。
2. loader、linker与Host activation保留精确deployment，不把scoped key退化成裸selector。
3. 同一active assembly中的同`serviceId + contractVersion`多revision、旧代、歧义和跨deployment
   替换全部fail closed。
4. Runtime frame Rust consumer适配v2携带的精确deployment，并验证该deployment与assembly binding一致。
   Router生产方属于并行R节点，不在本任务实现。
5. 不增加legacy compatibility、dual-read、fallback或ambient Host推导。

## 4. Evidence

先建立真实RED，再实现GREEN。至少覆盖：

- 两个不同service拥有相同`GET /v1/models`并成功装配、加载、链接；
- 同一精确deployment重复selector失败；
- 同service/version多revision失败；
- exact deployment在loader/linker/activation中保持一致；
- runtime frame跨deployment替换失败；
- old generation/wire失败。

运行相关locked聚焦测试、`cargo fmt --all -- --check`、`git diff --check`与旧裸selector owner反搜。
不得运行stable/live/network/Mongo/OAuth/browser，不push。

独立checkpoint只提交不依赖Router wire生产方的resolver、loader、linker与Host内部scoped-key
迁移。Host request wire consumer等待R节点的exact deployment frame先合入integration后，再从新
baseline做有界continuation；禁止把R提交混入本节点checkpoint。
