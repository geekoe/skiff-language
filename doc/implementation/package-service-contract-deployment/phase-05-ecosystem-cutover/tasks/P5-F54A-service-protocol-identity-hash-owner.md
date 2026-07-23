# P5-F54A：Service Protocol Identity Hash Owner

只改artifact-identity的typed service protocol hash helper/export/tests，以及runtime/loader identity委托与聚焦测试。
canonical helper严格解析v2 prefix+64 lowercase hex；loader删除本地grammar。覆盖canonical正例、wrong prefix、
缺/短/大写/nonhex、contractHash mismatch及published record回归。禁止fixture绕过或第二套parser。
运行两crate聚焦测试/check/rustfmt/diff，提交单一commit；禁止I02/R05/full gate。
