# P5-F347 External ingress compiler / artifact audit

状态：Ready（只读）。

## 直接父节点

- `P5-H35-external-ingress-surface-separation.md`

## 目标

只读追踪当前`service.yml -> compiler input -> PackageArtifact/ServiceContract ->
ServiceDeployment`全链，定位external ingress首次被错误提升为service operation的位置，并给出一个
canonical shared model/identity checkpoint的精确owner与后续compiler/deployment DAG。

必须回答：

1. 当前`ServiceManifestAuthoring`、route parser、service API projection和deployment generator怎样把
   `operation`解析为`ContractOperationId`。
2. 当前production是否已有可复用的typed handler/adapter/gateway entry DTO与Rust identity owner；若没有，
   最小新owner应在哪里，禁止复制Router TypeScript manifest模型。
3. 非public source callable是否已有稳定`PackageCallableId`、implementation link和完整linked signature；
   若缺失，首次丢失在哪一阶段。
4. `service.yml`目标shape怎样复用权威gateway adapter模型，并在compile input边界严格拒绝旧
   `operation`入口。
5. ServiceContract/ServiceProtocolIdentity、ServiceDeployment identity/generation和PackageArtifact
   generation各需怎样变化；不得保留兼容dual path。
6. HTTP raw/typed/stream、WebSocket connect/receive各自的最小production与负向探针。

## 范围与写入

只读检查`artifact-model`、`artifact-identity`、`compiler/**`、`deployment/**`及相关tests/fixtures。
不得修改production/test/corpus/lockfile。

只允许新增：

- `P5-F347-external-ingress-compiler-artifact-audit-result.md`

result记录exact commit/tree、关键跳点、首次损失、可复用owner、必要generation变化、写入冲突和建议DAG。
不运行workspace/stable/live，不push。提交result并返回commit。

## Worktree

- `/Users/geek/workspace/skiff-p5-f347-ingress-compiler-audit`
- `codex/p5-f347-ingress-compiler-audit`

