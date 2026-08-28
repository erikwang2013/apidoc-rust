//! 跨 crate 收集测试用的注解夹具：与测试 crate 中同名 fn 的注解
//! 一起链接进测试二进制，验证分布式切片跨 crate 合并与 id 隔离。

#[allow(dead_code)]
#[apidoc::title("fixture crate ping")]
#[apidoc::url("/fixture/ping")]
#[apidoc::method("GET")]
pub fn ping() {}
