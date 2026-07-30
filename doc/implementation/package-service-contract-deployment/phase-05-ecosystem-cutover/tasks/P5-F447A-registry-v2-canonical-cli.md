# P5-F447A Registry v2 And Canonical CLI

## Parent

[`P5-F447-managed-dev-watch-convergence.md`](P5-F447-managed-dev-watch-convergence.md)

## Scope

- 把registry硬切到`skiff-package-service-dev-registry-v2`；
- entry持久canonical `kind/root/serviceId?`，结构读取不访问live root；
- add执行live classify；remove按规范化root或持久service ID唯一匹配，歧义fail closed；
- 同目录temp file完整写入、file fsync、atomic rename，并在平台支持时同步父目录；
- canonical CLI只保留`skiff service dev registry add|list|remove`；
- 更新CLI usage、README与registry/tooling tests，删除v1及`skiff dev registry`兼容路径。

本任务不修改watch retry/CAS状态机，不启动stable instance。

## Evidence

- missing root仍可list/remove，add missing root失败；
- service ID唯一删除与root/service-ID歧义负例；
- malformed/duplicate/relative root/service ID drift负例；
- writer中途失败保留原文件，成功结果可重新严格读取；
- scripts type-check、聚焦CLI tests、legacy反向搜索与`git diff --check`。
