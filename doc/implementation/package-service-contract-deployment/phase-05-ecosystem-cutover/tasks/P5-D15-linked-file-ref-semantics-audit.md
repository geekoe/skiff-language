# P5-D15：Linked File Ref Semantics Audit

## 角色与结论

R13 PASS的combined tree已让std 11/11通过，checked-in package-service-host generation-2在业务请求前以
`assembly_admission_failed stage=link`拒绝。D15只读追踪authoring record、base/overlay link plan、production loader/
linked-program与Host admission；不得编辑、提交、修复或给F04 verdict。

结论为`DESIGN GO`：helper callable target与loaded FileIR的identity/module/source hash完全相同，唯一差异是顶层
FileIrRef带storage `artifactPath`、嵌套target为None。artifactPath按identity合同只是locator，不属于semantic ref；
`SharedPackageCode`的callable validation与`executable_addr`却使用完整FileIrRef相等，错误fail closed为
`CallableTargetFileRefMismatch`。

production authoring、artifact identity、loader/content validation、base/overlay code slots、helper same-heap package binding、
payments detached service binding均正确。仅在临时store去除顶层locator的只读等价探针中，同一assembly identity已成功
prepare/commit并真实输出checked-in consumer PASS；该探针只证明修复边界，不替代后续原样gate。

冻结F14/R14：只在linked-program semantic matcher忽略artifactPath，同时保留identity/module/present source hash/index
全部严格检查；两处production比较复用同一helper。不得把test-runner normalizer扩到external closure，也不得回改
authoring/identity/loader/fixture。
