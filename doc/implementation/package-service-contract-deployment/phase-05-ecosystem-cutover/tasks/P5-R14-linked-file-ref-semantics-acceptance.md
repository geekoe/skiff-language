# P5-R14：Linked File Ref Semantics Acceptance

未参与F14实现的独立只读Agent。输入为F14 exact clean commit/tree、D15/F14合同与R13 PASS combined tree；不得编辑、
提交、修复或给F04/R02 verdict。

必验：

- callable validation与`executable_addr`只复用一个semantic matcher；仅忽略artifactPath，identity/module/present hash/index
  与diagnostic仍严格；
- storageful/pathless正例真实进入production linker，所有负例仍fail closed；
- 未修改authoring/identity/loader/admission/test normalizer/fixture/Router/shared wire/manifest/lock，无record patch/re-sign；
- package/service link plan、same-heap/detached语义未被matcher旁路；
- 运行F14全部门禁，`extra-review`检查无第二matcher、宽泛PartialEq替代或test-only production分支。

第一行只给`R14 PASS`或`R14 FAIL`。PASS只允许exact合流后由原gate owner原样运行F04真实suite；D15临时probe、
linker单测或日志字符串均不能替代最终Host结果。

## 验收记录

首次`R14 FAIL`仅因candidate越界修改test fixture。same-base F14A `629f1c815f16c366c67557dfaba01a09455207fd` /
tree `5401add98ed9513fd495dd3eba4ac92e7ef3bce2`将构造逻辑移入允许的direct assembly test，fixture逐字恢复base；
single clean/lock、唯一matcher三处复用、production linker正例与严格负例全部保持，窄复验`R14 PASS`。
