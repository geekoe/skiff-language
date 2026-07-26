# P5-F376 Codex Relay HTTP gateway resume blocker

状态：TASK_SCOPE_EXPANDED（Relay source receiver authoring）。

## 已完成的安全checkpoint

- worktree：`/Users/geek/workspace/internals-p5-f367-relay-http-gateway`
- branch：`codex/p5-f367-relay-http-gateway`
- gateway/API/receipt static迁移checkpoint：
  `c0aa78cc1d1e601b43d3f5b2eba218a3aa6070c4`
- F368 package marker前置cherry-pick：
  `66afed285160e2a110850ccc9407cfe49e15e86c`
- final tree：`3213658638812013023b278e068209a6fbb1f76e`
- worktree clean；静态receipt测试4/4。

## 新阻塞

fresh isolated chain中std、llm-api与llm-providers均发布成功；Relay在artifact/receipt生成前lower失败：

```text
codex-relay/service/relay.skiff:5-7
interface CodexRelayProxyClient methods lack first `self: Self`
```

`relay.skiff:38-45`的具体实现已经有receiver。现有语言规则要求object-safe public instance interface
method显式声明receiver；因此这是Relay source authoring缺口，不需要改变compiler receiver语义。但
`relay.skiff`在F367/F376中明确禁止写入，必须新建节点授权，不能由续作任务顺手修改。

## 部分真实receipt

使用Skiff `a708c5abc41d285a8eb4c70734ebbb0f2f2efdee` /
`4a96ca557097d32ca85b8d5ad7e53432a0bf269c`：

- std：build `1828acd…4796`，Local ABI `c8be1d…b0ea`；
- llm-api：build `e8cf8c…b89c`，Local ABI `2ee60e…af7d`；
- llm-providers：build `08e9d0…5ad19`，Local ABI `b2e0c7…2057`；
- Relay：无receipt。

后继节点必须从clean `66afed2`继续，修正显式receiver后使用另一fresh store完成F367全部真实验收。
