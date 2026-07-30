# P5-D92：Package-owned Boundary Type 设计评审

状态：Ready（只读）

## 输入

- 唯一权威设计：`doc/architecture/package-service-contract-deployment.md` 当前未提交修改。
- 同步参考：
  - `doc/reference/publication.md`
  - `doc/reference/static-semantics.md`
  - `doc/reference/std-surface.md`
- 用户决策：Service首先是Package；类型统一由Package拥有。ServiceContract引用Package类型，不复制或重建
  service-owned类型；兼容性由类型内容身份区分。

## 评审目标

- 检查四对象模型与PackageSchemaTypeRecord/PackageSchemaIndex子记录是否自洽。
- 检查PackageSchemaTypeId/PackageSchemaIndexIdentity的owner、preimage、nameability和version/build排除项。
- 检查ServiceContract closure、consumer解析、service升级兼容性、protocol identity与fail-closed规则。
- 搜索仍与service-owned type或Contract内嵌复制模型冲突的canonical文档条款。
- 给出PASS或blocking findings；只读，不修改代码/文档，不讨论实现偏好。
