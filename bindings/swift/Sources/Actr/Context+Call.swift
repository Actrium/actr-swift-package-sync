import Foundation

public extension Context {
    func call(target: ActrId, routeKey: String, payload: Data) async throws -> Data {
        return try await callRaw(
            target: target,
            routeKey: routeKey,
            payloadType: .rpcReliable,
            payload: payload,
            timeoutMs: 30000
        )
    }
}
