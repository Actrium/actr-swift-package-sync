import ActrBindings

/// Re-export commonly used types so applications can `import Actr`.
public typealias Context = ContextBridge
public typealias RpcEnvelope = RpcEnvelopeBridge
public typealias Workload = WorkloadLifecycleBridge
public typealias DataStream = ActrBindings.DataStream
public typealias DataStreamCallback = ActrBindings.DataStreamCallback
public typealias MediaSample = ActrBindings.MediaSample
public typealias MediaType = ActrBindings.MediaType
public typealias MediaTrackCallback = ActrBindings.MediaTrackCallback
public typealias OpusEncoder = ActrBindings.OpusEncoder
public typealias LogCallback = ActrBindings.LogCallback

/// Resolve the package's own ActrType from a manifest.toml file.
public func resolveManifestPackageActrType(manifestPath: String) throws -> ActrType {
    try ActrBindings.resolveManifestPackageActrType(manifestPath: manifestPath)
}

/// Resolve a dependency ActrType from a manifest.toml file.
public func resolveManifestDependency(manifestPath: String, dependencyAlias: String) throws -> ActrType {
    try ActrBindings.resolveManifestDependency(
        manifestPath: manifestPath,
        dependencyAlias: dependencyAlias
    )
}

/// Creates a linked-runtime workload from lifecycle/dispatch and optional observers.
public func dynamicWorkload(
    lifecycle: Workload,
    signaling: SignalingObserverBridge? = nil,
    websocket: WebSocketObserverBridge? = nil,
    webrtc: WebRtcObserverBridge? = nil,
    credential: CredentialObserverBridge? = nil,
    mailbox: MailboxObserverBridge? = nil
) -> DynamicWorkload {
    DynamicWorkload(
        lifecycle: lifecycle,
        signaling: signaling,
        websocket: websocket,
        webrtc: webrtc,
        credential: credential,
        mailbox: mailbox
    )
}

/// Creates package-backed runtime observers without requiring workload dispatch callbacks.
public func runtimeObservers(
    signaling: SignalingObserverBridge? = nil,
    websocket: WebSocketObserverBridge? = nil,
    webrtc: WebRtcObserverBridge? = nil,
    credential: CredentialObserverBridge? = nil,
    mailbox: MailboxObserverBridge? = nil
) -> RuntimeObservers {
    RuntimeObservers(
        signaling: signaling,
        websocket: websocket,
        webrtc: webrtc,
        credential: credential,
        mailbox: mailbox
    )
}
