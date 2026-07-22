# P5-F25B：Object Construct Lowering

依赖F25A exact commit合流。独占`compiler/lowering/**`及直接IR/runtime-value tests，不回改source facts、runtime boundary、
fixture或业务ABI。独立worktree/branch，一个clean commit。

lowering只消费F25A materialization fact：record/discriminated-union按canonical field order递归生成`ExprIr::Construct`并包含
SyntheticNull；nested nominal Context/policy同样Construct。只有fact明确为Map/Json才生成MapLiteral；不得根据字符串name或
再次推断union branch。验证interpreter heap shape时只用现有eval test-support，不修改Runtime production。

tests覆盖accept/reject IR、nullable fill、nested Context/policy Object、Map/Json仍为Map、target facts缺失fail closed及现有
object literal回归。运行lowering/source接口精确tests、compiler checks、fmt/diff-check；禁止real smoke/full/I16/Host/stable。
