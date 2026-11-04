//! 渲染层：宠物本体的平台相关绘制（Windows 分层窗口）。
//! 档1 基线为 UpdateLayeredWindow；档3 DirectComposition + D3D11 后续并列加入。

pub mod dcomp;
pub mod layered;
pub mod win32;
