# Phase 04：ServiceDeployment Projection

状态：outline-only；Phase 03 验收后再细化

## 输入

- ServiceContract artifact。
- Provider PackageArtifact closure及其 boundary projections/requirements。

## 目标

- 冻结 source-free deployment authoring 输入。
- 显式映射 contract operation id 到 package callable id，Ingress只映射 contract operation。
- 仅凭 typed artifacts 校验 signature/schema、effect guarantees、config/state/resource/capability bindings。
- 生成独立 deployment revision/identity；provider package build、config和route变化不污染 protocol identity。
- 删除 service source compile和 code-owning ServiceUnit production path。

## 验收边界

- deployment projection 不读取 AST、source text或 lowering helper。
- operation缺失、额外、重复、boundary unavailable、effect不满足或 requirement未绑定均在 projection
  阶段 fail closed。
- ServiceDeployment 不拥有全局 provider closure resolution；只保存 dependency selector/constraints。
- 本阶段不要求 runtime 执行新 deployment。

## 细化前复查

复查 service.yml/profile、config activation、state/DB owner、ingress projection、artifact writer和旧
ServiceUnit consumers。直接重叠的配置/校验逻辑必须先收敛。
