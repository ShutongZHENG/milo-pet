//! milo-pet 纯逻辑内核：物理、积分、帧包、配置、渲染抽象。
//! 不依赖任何平台 API（无 windows crate），可独立单元测试。

pub mod config;
pub mod frames;
pub mod physics;
pub mod render;
pub mod score;
