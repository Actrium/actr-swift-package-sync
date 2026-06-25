import ActrBindings

/// Re-export commonly used types so applications can `import Actr`.
public typealias Context = ContextBridge
public typealias RpcEnvelope = RpcEnvelopeBridge
public typealias ErrorEvent = ErrorEventBridge
public typealias Workload = WorkloadLifecycleBridge
public typealias SignalingObserver = SignalingObserverBridge
public typealias WebSocketObserver = WebSocketObserverBridge
public typealias WebRtcObserver = WebRtcObserverBridge
public typealias CredentialObserver = CredentialObserverBridge
public typealias MailboxObserver = MailboxObserverBridge
public typealias DataStream = ActrBindings.DataStream
public typealias DataStreamCallback = ActrBindings.DataStreamCallback
public typealias MediaSample = ActrBindings.MediaSample
public typealias MediaType = ActrBindings.MediaType
public typealias MediaTrackCallback = ActrBindings.MediaTrackCallback
public typealias OpusEncoder = ActrBindings.OpusEncoder
public typealias LogCallback = ActrBindings.LogCallback

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
