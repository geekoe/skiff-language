# P5-F445H-I7-M4 final dynamic readiness result

状态：

```text
M4_EXECUTION=CLOSED_WITH_BLOCKER
RELAY_ISOLATED=PASS
I7_M_COMPLETE=NO
C_COMPLETE=NO
A_COMPLETE=NO
J_EXECUTED=NO
J_COMPLETE=NO
U_UNBLOCKED=NO
DECISION_REQUIRED=NO
BLOCKING_CLASSES=3
```

本ledger是M3、C2、A与J旧readiness result在当前final候选上的统一successor。它保留已经
GREEN的结构、graph、receipt与Relay isolated证据，同时以AIHub真实52项执行结果替代旧的
compile/link、D7 `409`或20秒prepare `504`首错。

M4没有完成整个M：AIHub已经进入全部真实case，但仍有39项动态失败；按gate合同在发现
production/runtime blocker后停止，因此Agine 170项与J均未执行。

## 1. Frozen identities

| Repository | Commit | Tree |
| --- | --- | --- |
| Skiff integration ledger baseline | `f099fb7189d6ed5b49de225fd145e01c341079b6` | `b98b4cdecf8b5c400720e064f74a34356aa8af3c` |
| Internals integration final | `9c3bdc82c4a43e575ea627357c05f54dbc0400a8` | `c3f159a397cd3c2b316a502ce945d8a935a9c2c3` |
| official packages candidate | `b06d7aaf16b6914837de1f74920fd3f626040472` | `fb9db28a7d1bd3babafd1dfa7a23687e393ff856` |

Internals final相对其第一父的M4机械diff只有：

- `codex-relay/service-tests/config.skiff-test.yml`删除未声明的`cookieName`与
  `maxAgeSeconds`；
- `agine/service-tests/config.skiff-test.yml`删除同样两个未声明binding；
- `codex-relay/service-tests/relay_routes.test.skiff`删除非法的
  `.exception.error.reason`普通成员访问，保留opaque exception tag断言。

精确统计为3个test-only文件、7行删除。test case、dependency edge、dependency-only base、
collection mapping与production source均未改变。

## 2. Static, graph and receipt evidence

T0结构门禁为`27/27 PASS`。三个exact service graph与canonical receipt均PASS：

| Target | 用时 | 结果 |
| --- | ---: | --- |
| Relay | `29.07s` | PASS |
| AIHub | `33.89s` | PASS |
| Agine | `69.09s` | PASS |

graph在M4机械diff之前执行，但该证据没有失效：后续7行删除只位于test roots，而graph只消费
production target。若后续owner修改production target或dependency closure，则必须重新生成相应
graph/receipt，不能沿用本条豁免。

dependency-only base继续冻结为：

| Target | Base assembly deployments |
| --- | --- |
| Relay | 无 |
| AIHub | 仅Relay |
| Agine | 仅Relay与AIHub |

target自身不进入base；config只来自各自`config.skiff-test.yml`；没有启用`includeTarget`。
因此本轮没有用target deployment或production config掩盖dependency-only边界。

## 3. M: isolated execution

### 3.1 Relay

Relay的执行序列为：

1. `96.91s`：旧test config因未声明binding fail closed；
2. 删除无效config后，`173.48s`真实执行75项，`74 passed / 1 failed`；唯一失败是旧
   opaque exception成员断言；
3. 删除非法`.exception.error.reason`访问并保留tag断言后，`174.84s`，
   `75/75 PASS`、`0 skipped`。

因此：

```text
RELAY_ISOLATED=PASS
```

### 3.2 AIHub

AIHub最终真实isolated run用时`206.17s`，执行52项：

```text
13 passed
39 failed
0 skipped
```

该run没有再出现以下旧blocker：

- 20秒activation prepare `504`；
- D7之前的multiple collection projection `409`；
- foreign DB compiler/linker/Eval exact-identity错误；
- P4/P4B canonical public type或top-level callable alias错误。

当前失败由三类精确blocker组成：

1. Skiff native generic plan：`unsupported native target std.json.encode`；
2. Skiff Stream runtime value：`unknown Stream value`；
3. Internals opaque Exception production consumer：
   `request-local exception does not support ordinary member access`。

Internals final中`aihub/service/internal/aihub_service.skiff`仍有15处
`.exception.error` production访问，test root另有9处。修改test root不能遮蔽production consumer，
而M4没有权限修改production，因此不能把AIHub记为GREEN。

### 3.3 Agine stop discipline

发现AIHub production/runtime blocker后，按合同停止；Agine 170项未运行。Agine graph与receipt
PASS只证明静态候选可构造，不替代isolated assertions。

因此：

```text
M4_EXECUTION=CLOSED_WITH_BLOCKER
I7_M_COMPLETE=NO
```

## 4. C and A successor verdicts

### 4.1 C

Relay provider isolated matrix已经`75/75 PASS`，但AIHub caller/combined侧仍为
`13 passed / 39 failed`。C要求Relay与AIHub在同一final候选上均完成真实isolated assertions，
所以：

```text
C_COMPLETE=NO
```

旧C2的20秒prepare `504`已被更深的真实执行证据取代；它不再是current first blocker。

### 4.2 A

Agine exact graph与receipt PASS，旧未声明config binding也已从test-only config删除；但本轮因
AIHub blocker按顺序停止，Agine 170项没有执行。因此：

```text
A_COMPLETE=NO
```

不能用graph/receipt或Relay GREEN代替Agine positive/negative isolated matrix。

## 5. J and U readiness

C与A均未完成，所以U没有解锁，J也没有执行：

```text
U_UNBLOCKED=NO
J_EXECUTED=NO
J_COMPLETE=NO
```

本ledger不是J execution result，也没有运行final hermetic join。official packages identity只是
冻结J的package候选，不代表J已经消费或验收该候选。

## 6. Required next owners

后续必须分别关闭：

1. Skiff native generic plan对`std.json.encode`的runtime target支持；
2. Skiff Stream value的runtime表示与消费；
3. Internals AIHub production中的opaque Exception普通成员访问。

三个owner完成后，应冻结同一组新的Skiff、Internals与official packages final identities；若
production target或dependency closure变化，先重跑受影响的exact graph/receipt，再从AIHub同一
52项isolated matrix开始复验。AIHub全绿后按顺序运行Agine 170项；只有C与A都完成，才能解锁U并
执行唯一J final hermetic join。

上述工作不需要产品决策：

```text
DECISION_REQUIRED=NO
```

本提交只新增该result ledger，不修改production、tests或旧result，也没有运行测试、stable/live、
network、MongoDB、OAuth、browser或push。
