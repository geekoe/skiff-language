# P5-F160：Package Schema Content-addressed Store

状态：Ready

## 直接父任务

- `P5-F159-package-schema-artifact-model-result.md`

## 范围

只修改artifact ecosystem path与canonical artifact store。不得修改compiler projection、compiler input、
deployment projection或runtime。

## 必须实现

- 为`PackageSchemaIndexIdentity`与`PackageSchemaTypeId`定义独立canonical record path。
- `CanonicalArtifactStore`支持严格写入/读取`PackageSchemaIndex`和`PackageSchemaTypeRecord`。
- 写入前调用canonical identity校验；同identity同内容可幂等，已有不同内容必须拒绝。
- 读取时重新校验path identity、record identity、package owner、stable key和descriptor hash。
- 不增加release pointer、version selector或第五种发布对象。
- 提供一次PackageArtifact schema refs的完整解析入口；缺index/type record、index entry与ref不一致、
  多余或缺失artifact record ref均fail closed。该入口只返回已验证记录，不做compiler/runtime语义。

## 验证

- store聚焦测试覆盖round-trip、幂等、冲突、缺失、错path、错owner、错hash、index/ref不一致。
- 两个PackageArtifact或ServiceContract可解析到同一个type record，不产生重复payload。
- `git diff --check`；独立提交并写result。

