//! TURN 认证相关工具（保留模块以备扩展）。
//!
//! TurnCredential（HMAC 时效凭证）由服务端生成，直接通过 Proto 下发，
//! 无需客户端本地构造，因此本模块不再包含 Claims 等编码逻辑。
