# P5-F380 Relay interface receiver and gateway completion blocker

状态：TASK_SCOPE_EXPANDED（interface suspension projection shared owner）。

## 已完成checkpoint

在clean F376 checkpoint上，只给`CodexRelayProxyClient`的两个instance method增加了首参数
`self: Self`：

- commit：`68c7d679899bf942060fe407270cea60b7ba85ca`
- tree：`de19e938259e6f023cd206791f4bfb9c5e4d03d9`
- worktree/branch：
  `/Users/geek/workspace/internals-p5-f367-relay-http-gateway` /
  `codex/p5-f367-relay-http-gateway`
- worktree clean；既有checkpoint `c0aa78c`与`66afed2`未改写。

receiver修复本身符合现有object-safe interface规则，compiler source tests 10/10、service-call-root tests
2/2及Relay静态receipt 4/4通过。

## 新阻塞

使用Skiff
`b240c380dd1535d6405d39337694f53e4d385ce7` /
`4751efdf0a94410b9620795851429a2875aa3f2f`和fresh store发布：

```text
std -> llm-api -> llm-providers -> Relay
```

前三包成功；Relay在PackageArtifact identity验证失败：

```text
public instance relayProxy method responsesCompleted
return or suspension semantics disagree with its interface
```

最小临时诊断保持receiver和声明return不变，只把实现体换成纯`{}`即能生成2-operation contract。由此确认：

- 真实实现投影：`maySuspend = true`
- interface FileIR投影：`maySuspend = false`

这不是gateway或receipt问题。通过修改Relay业务实现消除挂起点会改变业务语义；F380禁止这样做。后继必须先
确定interface声明、compiler effect projection与public ABI中`maySuspend`的canonical owner。

## 保留状态

Relay的API/gateway/receiver改动已安全提交但尚未合入Internals integration。完成shared owner修复后必须从
当前clean `68c7d679`继续fresh发布，取得真实2/30/30 receipt后才能合流。
