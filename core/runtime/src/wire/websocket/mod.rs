//! WebSocket subsystem
//!
//! WebSocket Connection implementation

pub mod connection;
pub mod gate;
pub mod server;

pub use connection::WebSocketConnection;
pub use gate::{WebSocketGate, WsAuthContext};
pub use server::WebSocketServer;
