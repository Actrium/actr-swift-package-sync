import ActrBindings

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
    signaling: SignalingObserver? = nil,
    websocket: WebSocketObserver? = nil,
    webrtc: WebRTCObserver? = nil,
    credential: CredentialObserver? = nil,
    mailbox: MailboxObserver? = nil
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
    signaling: SignalingObserver? = nil,
    websocket: WebSocketObserver? = nil,
    webrtc: WebRTCObserver? = nil,
    credential: CredentialObserver? = nil,
    mailbox: MailboxObserver? = nil
) -> RuntimeObservers {
    RuntimeObservers(
        signaling: signaling,
        websocket: websocket,
        webrtc: webrtc,
        credential: credential,
        mailbox: mailbox
    )
}
