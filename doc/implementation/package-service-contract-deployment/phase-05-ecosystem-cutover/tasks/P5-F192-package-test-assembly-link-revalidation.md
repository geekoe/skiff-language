# P5-F192：Package Test Assembly Link 复验

状态：Ready

## 直接父任务

- `P5-F180L-actor-full-chain-acceptance-result.md`

## 问题与目标

真实 aliyunoss/http-session/openai/track PackageArtifact 均成功生成，但
`skiff test aliyunoss` 的隔离 Runtime 在 assembly link 阶段返回 HTTP 409，测试工具未保留精确
link 诊断。先让 isolated test 可观测底层拒绝原因，再修 canonical package-test
artifact/deployment/assembly/link 链。

不得修改 package 源码绕过，不得吞掉 Runtime link error。

## 验证

- aliyunoss、http-session、openai、track 真实 `skiff test`；
- package dependency 与 wrong-ref fail-closed；
- package-test/loader/linker/host 聚焦测试；
- workspace check、diff check；
- 独立提交和 result。

