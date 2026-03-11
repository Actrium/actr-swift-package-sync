import Foundation
import SwiftProtobuf

/// Associates an RPC request message with its response type and routing metadata.
///
/// ## Conformance Example
///
/// ```swift
/// struct EchoRequest: Message, Sendable {
///     var message: String = ""
/// }
///
/// struct EchoResponse: Message, Sendable {
///     var reply: String = ""
/// }
///
/// extension EchoRequest: RpcRequest {
///     typealias Response = EchoResponse
///     static var routeKey: String { "echo.EchoService.Echo" }
/// }
/// ```
public protocol RpcRequest: Message, Sendable {
    /// The response type associated with this RPC request.
    associatedtype Response: Message

    /// Returns the route key for this RPC method.
    ///
    /// Format: "package.Service.Method" (e.g., "echo.EchoService.Echo")
    static var routeKey: String { get }
}
