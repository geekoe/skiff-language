# P5-R446 Unified Config And Service DB Acceptance

## Role

独立只读验收F446A–D exact integration candidates。先核验共同验收矩阵与跨仓库commit/tree，再按风险选择
聚焦probe；同一代码状态已有昂贵完整gate时不机械重跑。

## Blocking Checks

- canonical reference/architecture、shared DTO、producer、reader、Router、Runtime、tooling、test-runner与生态
  authoring使用同一snapshot/service DB语义；
- 反向搜索旧config literal/SecretRef/state binding/profile policy没有production、fixture、golden、sample或
  active task残留；
- config值不出现在四类artifact JSON、identity preimage、receipt、control frame、health或日志；
- activation exact-pair recovery、snapshot target environment在ConfigView物化前的strict比较、Package
  ConfigView隔离，以及storage-domain/environment/service-derived DB均有真实动态证据；
- `collection_name_mapping`在production authoring/schema/compiler/artifact/runtime/fixture中为零；logical
  collection identity只由provider DB declaration拥有，physical name由系统编码；
- runtime frame当前代际只有v3，旧v2 reader/writer/fixture均无兼容路径；
- secret文件未提交，mode正确，验收输出不泄漏内容；
- full non-live、stable cold activation和Agine chat smoke证据属于同一最终候选。

第一行输出`PASS`或`FAIL`。FAIL必须指出唯一owner、代码证据、失效gate与最小修复边界；不得在验收worktree
直接修复。
