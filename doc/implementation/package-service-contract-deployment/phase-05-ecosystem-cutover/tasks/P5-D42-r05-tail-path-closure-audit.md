# P5-D42：R05 Tail Path Closure Audit

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点D42，依赖R05与R05A在同一真实transcript连续暴露两个新blocker。根据收敛熔断，D42必须在第三次完整probe前
只读闭合从B unary response bytes到最终A release/drain的剩余production路径；不作R05/R02/Phase verdict。

全新只读Agent在exact production candidate
`8c832b44a49b31da393064ab2c6c7d432db70274`建立矩阵：

- B fixture的unary返回值如何经Runtime `RuntimePayload` canonical codec编码、Router原样响应，以及scripts/test
  infrastructure中是否已有唯一JS decode owner；列出可复用API、依赖方向和错误/limit语义；
- generation lifecycle harness当前body采集、oracle与I32 JSON fake的精确缺口，如何用canonical owner做最小direct
  evidence，禁止复制`SKPV` parser；
- B unary marker确认后，close B、release ACK、health pin回1、close A、pin/in-flight归0、pending activation清空的每个
  production跳点、owner、可观察字段及deadline；
- finally/best-effort close与正常close oracle的差异，cleanup失败是否可能遮挡lifecycle失败；
- 尚未执行或被上游遮挡的真实范围、关键负例、最小便宜诊断探针，以及第三次完整probe前应一次批量修复的所有
  implementation节点；
- 若正确canonical decode后仍可能存在fixture返回类型/HTTP gateway语义或public diagnostic缺口，区分
  implementation owner与必须升级的设计决策。

只允许`rg`、`git show/diff/log`及源码/fixture/既有测试静态读取；禁止编辑、提交、构建、测试、启动任何
Router/runtime/instance、运行transcript/旧smoke或操作stable。输出必须包含关键跳点与遮挡矩阵、唯一/分离写入owner、
direct tests、合流后cheap combined、证据失效面及第三次probe的解除条件。不得自行承接修复节点。
