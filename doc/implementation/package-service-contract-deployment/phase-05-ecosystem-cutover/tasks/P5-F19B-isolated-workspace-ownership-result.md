# P5-F19B：Isolated Workspace Ownership Result

`F19B PASS`

开发提交`c5f66ad281e847306af660a30af58c28fed746fd`，parent `67daaa18bd634ea83d07a9b6440a57de51265b45`，
tree `f67914c0bf438121de889b01e6da1082c0193de7`，lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。只修改合同exclusive isolated workspace/test路径。

- 单一owner以exclusive marker、128-bit nonce、root/marker/config dev+ino与realpath receipt拥有workspace；config为`wx`。
- down/status/remove前逐次复验；foreign/missing/corrupt/symlink replacement不调用foreign config、不删除foreign路径。
- primary-first且all-settled关闭owned supervisor/instance/ports/lease；`stopped:false`为cleanup failure。
- 12场景teardown矩阵及12个既有foreign sibling preservation通过；isolated test 30/30、runtime isolation 3/3、node check/
  diff check PASS。port lease foreign token replacement保留；未实证read→unlink并发窗口，维持非阻塞残余风险。

Extra-review确认ownership与cleanup step已抽为单一模块/owner，无第二recursive cleanup实现。未运行真实isolated runtime、
I16、H18、Host/full或stable。
