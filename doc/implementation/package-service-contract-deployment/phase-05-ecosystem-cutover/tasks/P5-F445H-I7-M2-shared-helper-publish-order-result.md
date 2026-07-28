# P5-F445H-I7-M2 Shared helper publish-order result

状态：

```text
PASS
M2_COMPLETE=YES
PUBLISH_ORDER_BLOCKER=CLOSED
M_STATUS=PARTIAL
I7_M_COMPLETE=NO
DECISION_REQUIRED=NO
BLOCKING_ISSUES=1
```

M2关闭了shared isolated helper的publish-order缺口。普通packages现在先发布，dependency services随后按
拓扑顺序发布并产生真实ServiceContract/deployment pointer，subject target最终仍只作为
`PackageArtifact`发布。

该修复使AIHub真实复测越过原先缺少Relay ServiceContract pointer的错误，完成dependency发布、编译、
链接并进入isolated runtime activation。复测随后在一个独立的collection projection歧义上返回`409`。
因此M2完成，但总体M仍为partial。

## 1. Frozen inputs

| 项 | 值 |
| --- | --- |
| Skiff tool candidate | `07090bc3b13025f4dfc24f6413bdf225010c56b1` |
| Internals baseline | `7fa2ac5de5a576013ee2be74032435a361c8a6e4` / `dcf91f0243e230ea5eff03f1f00ac2d7990d325b` |
| Internals implementation | `2016af5` |
| Skiff integration ledger anchor | `b4275a48548bb21b8294d089c9108f7142609b40` |

本result在后续Skiff integration HEAD上持久化，但只登记上述冻结候选与Internals证据，不把后续ledger
移动解释为工具或业务实现变化。

## 2. RED evidence

修复前执行：

```bash
node scripts/test-isolated-service.mjs agine.ai/aihub
```

在`22.5763s`失败：

```text
service dependency agine.ai/codex-relay@0.1.0 has no published ServiceContract pointer
```

失败发生在activation之前。原因是helper先在packages阶段发布带service dependency的AIHub subject，
而Relay ServiceContract要到后续services阶段才发布；这属于shared helper publish ordering/closure
问题，不是P1 timeout回归。

## 3. Minimal implementation result

Shared helper现在执行：

1. 先发布ordinary packages；
2. 按拓扑顺序发布dependency services，取得真实contract与deployment pointer；
3. 最后只以`PackageArtifact`发布subject `targetPackage`；
4. 用dependency service deployment构造dependency-only base assembly；
5. 最终target deployment继续由test package拥有。

AIHub的base assembly仍只包含Relay。Subject ordinary deployment不会进入base assembly，也没有伪造
ServiceContract/deployment pointer。三个业务test root均未修改。

## 4. Static and Node evidence

| 检查 | 结果 |
| --- | --- |
| shared Node tests | PASS，`27/27` |
| AIHub workflow guards | PASS，`21/21` |

约束复核确认：

- base assembly保持dependency-only；
- subject只发布package，不作为ordinary deployment混入base；
- 无pointer伪造；
- 无`includeTarget`或`config.dev`绕过；
- 无Relay、AIHub或Agine业务test root改动。

## 5. Real rerun and next blocker

AIHub真实复测已经：

- 越过缺少Relay ServiceContract pointer的原错误；
- 完成dependency publish；
- 完成test package compile与link；
- 进入isolated runtime activation。

最终返回：

```text
HTTP 409
AssemblyActivationRejected
multiple active collection projections for llmProviders
```

AIHub test package对`llmProviders`从两个dependency package identities形成active collection
projection歧义。这是M2之后的独立blocker，应由另案冻结identity选择与projection authority；不得回退
publish顺序、伪造pointer、把subject普通deployment塞入base assembly，或修改业务test root绕过。

## 6. Verdict

```text
M2_COMPLETE=YES
PUBLISH_ORDER_BLOCKER=CLOSED
M_STATUS=PARTIAL
I7_M_COMPLETE=NO
```

M2只关闭shared helper publish-order问题。新的collection projection阻塞尚未关闭，AIHub test
assertions也未据此宣告通过，因此不能把M或I7整体标为完成。

本result只修改Skiff integration ledger，没有修改Skiff工具代码，没有访问
stable/live/network/Mongo/OAuth/browser，也没有push。
