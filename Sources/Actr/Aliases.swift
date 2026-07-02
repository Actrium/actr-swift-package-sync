import ActrBindings

// MARK: - Core API

public typealias ActrId = ActrBindings.ActrId
public typealias ActrType = ActrBindings.ActrType
public typealias PayloadType = ActrBindings.PayloadType
public typealias ActrRefWrapper = ActrBindings.ActrRefWrapper
public typealias DynamicWorkload = ActrBindings.DynamicWorkload
public typealias RuntimeObservers = ActrBindings.RuntimeObservers

// MARK: - Application-facing bridge aliases

public typealias Context = ContextBridge
public typealias RpcEnvelope = RpcEnvelopeBridge
public typealias Workload = WorkloadLifecycleBridge
public typealias ErrorEvent = ErrorEventBridge
public typealias ErrorCategory = ErrorCategoryBridge
public typealias PeerEvent = PeerEventBridge
public typealias WebRTCPeerStatus = WebRtcPeerStatusBridge
public typealias CredentialEvent = CredentialEventBridge
public typealias BackpressureEvent = BackpressureEventBridge
public typealias SignalingObserver = SignalingObserverBridge
public typealias WebSocketObserver = WebSocketObserverBridge
public typealias WebRTCObserver = WebRtcObserverBridge
public typealias CredentialObserver = CredentialObserverBridge
public typealias MailboxObserver = MailboxObserverBridge

// MARK: - Low-level bridge aliases

public typealias ContextBridge = ActrBindings.ContextBridge
public typealias WorkloadLifecycleBridge = ActrBindings.WorkloadLifecycleBridge
public typealias RpcEnvelopeBridge = ActrBindings.RpcEnvelopeBridge
public typealias ErrorEventBridge = ActrBindings.ErrorEventBridge
public typealias ErrorCategoryBridge = ActrBindings.ErrorCategoryBridge
public typealias PeerEventBridge = ActrBindings.PeerEventBridge
public typealias WebRtcPeerStatusBridge = ActrBindings.WebRtcPeerStatusBridge
public typealias CredentialEventBridge = ActrBindings.CredentialEventBridge
public typealias BackpressureEventBridge = ActrBindings.BackpressureEventBridge
public typealias SignalingObserverBridge = ActrBindings.SignalingObserverBridge
public typealias WebSocketObserverBridge = ActrBindings.WebSocketObserverBridge
public typealias WebRtcObserverBridge = ActrBindings.WebRtcObserverBridge
public typealias CredentialObserverBridge = ActrBindings.CredentialObserverBridge
public typealias MailboxObserverBridge = ActrBindings.MailboxObserverBridge

// MARK: - Networking and lifecycle

public typealias NetworkEventHandleWrapper = ActrBindings.NetworkEventHandleWrapper
public typealias NetworkAvailability = ActrBindings.NetworkAvailability
public typealias NetworkTransportFlags = ActrBindings.NetworkTransportFlags
public typealias NetworkSnapshot = ActrBindings.NetworkSnapshot
public typealias AppLifecycleState = ActrBindings.AppLifecycleState
public typealias CleanupReason = ActrBindings.CleanupReason
public typealias ReconnectReason = ActrBindings.ReconnectReason
public typealias NetworkEvent = ActrBindings.NetworkEvent
public typealias NetworkEventResult = ActrBindings.NetworkEventResult

// MARK: - Errors

public typealias ActrError = ActrBindings.ActrError
public typealias ConnectionNotReadyInfo = ActrBindings.ConnectionNotReadyInfo
public typealias ErrorKind = ActrBindings.ErrorKind

// MARK: - Streams and media

public typealias DataStream = ActrBindings.DataStream
public typealias DataStreamCallback = ActrBindings.DataStreamCallback
public typealias MediaSample = ActrBindings.MediaSample
public typealias MediaType = ActrBindings.MediaType
public typealias MediaTrackCallback = ActrBindings.MediaTrackCallback
public typealias OpusEncoder = ActrBindings.OpusEncoder
public typealias LogCallback = ActrBindings.LogCallback
