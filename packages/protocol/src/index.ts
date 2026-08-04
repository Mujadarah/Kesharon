export const PROTOCOL_VERSION = 1 as const;

export type HealthStatus = "ready" | "degraded";

export type ErrorCode =
  | "invalidRequest"
  | "notGitRepository"
  | "projectPathUnavailable"
  | "operationCancelled"
  | "requestInProgress"
  | "serverBusy"
  | "internalError";

export interface HealthSnapshot {
  requestId: string;
  status: HealthStatus;
  protocolVersion: typeof PROTOCOL_VERSION;
}

export interface ProjectSnapshot {
  id: string;
  displayName: string;
  canonicalRoot: string;
  trusted: boolean;
}

export interface OperationSnapshot {
  requestId: string;
  kind: "openProject";
  status: "running";
}

export interface WorkspaceSnapshot {
  streamId: string;
  lastSequence: number;
  project: ProjectSnapshot | null;
  activeOperations: OperationSnapshot[];
}

export type DaemonEventPayload =
  | { type: "operationStarted"; requestId: string; kind: "openProject" }
  | { type: "projectOpened"; requestId: string; project: ProjectSnapshot }
  | { type: "operationCancelled"; requestId: string }
  | {
      type: "operationFailed";
      requestId: string;
      code: ErrorCode;
      message: string;
    };

export interface DaemonEvent {
  protocolVersion: typeof PROTOCOL_VERSION;
  streamId: string;
  sequence: number;
  payload: DaemonEventPayload;
}

export type StreamMessage =
  | { messageType: "snapshot"; snapshot: WorkspaceSnapshot }
  | { messageType: "event"; event: DaemonEvent };

export class ProtocolDecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProtocolDecodeError";
  }
}

export class DaemonResponseError extends ProtocolDecodeError {
  constructor(
    public readonly code: ErrorCode,
    message: string
  ) {
    super(message);
    this.name = "DaemonResponseError";
  }
}

export type ResponsePayload =
  | {
      type: "health";
      status: HealthStatus;
      protocolVersion: typeof PROTOCOL_VERSION;
    }
  | { type: "projectOpened"; project: ProjectSnapshot }
  | {
      type: "cancellation";
      targetRequestId: string;
      outcome: "accepted" | "alreadyFinished" | "notFound";
    }
  | { type: "subscriptionReady"; streamId: string };

export interface DaemonErrorPayload {
  code: ErrorCode;
  message: string;
}

export type DaemonResponse =
  | {
      protocolVersion: typeof PROTOCOL_VERSION;
      requestId: string;
      result: ResponsePayload;
      error: null;
    }
  | {
      protocolVersion: typeof PROTOCOL_VERSION;
      requestId: string;
      result: null;
      error: DaemonErrorPayload;
    };

export function decodeDaemonResponse(value: unknown): DaemonResponse {
  if (!isRecord(value)) {
    throw new ProtocolDecodeError("Malformed daemon response");
  }
  requireProtocolVersion(value);
  if (typeof value.requestId !== "string") {
    throw new ProtocolDecodeError("Malformed daemon response");
  }
  if (value.error === null && isResponsePayload(value.result)) {
    return value as unknown as DaemonResponse;
  }
  if (value.result === null && isDaemonErrorPayload(value.error)) {
    return value as unknown as DaemonResponse;
  }
  throw new ProtocolDecodeError("Malformed daemon response");
}

export function decodeHealthResponse(value: unknown): HealthSnapshot {
  const response = decodeDaemonResponse(value);
  const result = requireResult(response);
  if (result.type !== "health") {
    throw new ProtocolDecodeError("Malformed daemon response");
  }

  return {
    requestId: response.requestId,
    status: result.status,
    protocolVersion: result.protocolVersion
  };
}

export function decodeProjectOpenResponse(value: unknown): {
  requestId: string;
  project: ProjectSnapshot;
} {
  const response = decodeDaemonResponse(value);
  const result = requireResult(response);
  if (result.type !== "projectOpened") {
    throw new ProtocolDecodeError("Malformed daemon response");
  }
  return { requestId: response.requestId, project: result.project };
}

export function decodeCancellationResponse(value: unknown): {
  requestId: string;
  targetRequestId: string;
  outcome: "accepted" | "alreadyFinished" | "notFound";
} {
  const response = decodeDaemonResponse(value);
  const result = requireResult(response);
  if (result.type !== "cancellation") {
    throw new ProtocolDecodeError("Malformed daemon response");
  }
  return {
    requestId: response.requestId,
    targetRequestId: result.targetRequestId,
    outcome: result.outcome
  };
}

export function decodeSubscriptionReadyResponse(value: unknown): {
  requestId: string;
  streamId: string;
} {
  const response = decodeDaemonResponse(value);
  const result = requireResult(response);
  if (result.type !== "subscriptionReady") {
    throw new ProtocolDecodeError("Malformed daemon response");
  }
  return { requestId: response.requestId, streamId: result.streamId };
}

export function decodeStreamMessage(value: unknown): StreamMessage {
  if (!isRecord(value)) {
    throw new ProtocolDecodeError("Malformed daemon stream message");
  }
  if (value.messageType === "snapshot" && isWorkspaceSnapshot(value.snapshot)) {
    return value as unknown as StreamMessage;
  }
  if (
    value.messageType === "event" &&
    isRecord(value.event) &&
    value.event.protocolVersion === PROTOCOL_VERSION &&
    typeof value.event.streamId === "string" &&
    Number.isSafeInteger(value.event.sequence) &&
    (value.event.sequence as number) > 0 &&
    isEventPayload(value.event.payload)
  ) {
    return value as unknown as StreamMessage;
  }
  throw new ProtocolDecodeError("Malformed daemon stream message");
}

function isProjectSnapshot(value: unknown): value is ProjectSnapshot {
  return (
    isRecord(value) &&
    typeof value.id === "string" &&
    typeof value.displayName === "string" &&
    typeof value.canonicalRoot === "string" &&
    typeof value.trusted === "boolean"
  );
}

function isWorkspaceSnapshot(value: unknown): value is WorkspaceSnapshot {
  return (
    isRecord(value) &&
    typeof value.streamId === "string" &&
    Number.isSafeInteger(value.lastSequence) &&
    (value.lastSequence as number) >= 0 &&
    (value.project === null || isProjectSnapshot(value.project)) &&
    Array.isArray(value.activeOperations) &&
    value.activeOperations.every(
      (operation) =>
        isRecord(operation) &&
        typeof operation.requestId === "string" &&
        operation.kind === "openProject" &&
        operation.status === "running"
    )
  );
}

function isEventPayload(value: unknown): value is DaemonEventPayload {
  if (!isRecord(value) || typeof value.requestId !== "string") {
    return false;
  }
  switch (value.type) {
    case "operationStarted":
      return value.kind === "openProject";
    case "projectOpened":
      return isProjectSnapshot(value.project);
    case "operationCancelled":
      return true;
    case "operationFailed":
      return isErrorCode(value.code) && typeof value.message === "string";
    default:
      return false;
  }
}

function requireProtocolVersion(value: Record<string, unknown>): void {
  if (value.protocolVersion !== PROTOCOL_VERSION) {
    const version =
      typeof value.protocolVersion === "number"
        ? String(value.protocolVersion)
        : "unknown";
    throw new ProtocolDecodeError(`Unsupported protocol version ${version}`);
  }
}

function requireResult(response: DaemonResponse): ResponsePayload {
  if (response.error !== null) {
    throw new DaemonResponseError(response.error.code, response.error.message);
  }
  return response.result;
}

function isResponsePayload(value: unknown): value is ResponsePayload {
  if (!isRecord(value)) {
    return false;
  }
  switch (value.type) {
    case "health":
      return (
        (value.status === "ready" || value.status === "degraded") &&
        value.protocolVersion === PROTOCOL_VERSION
      );
    case "projectOpened":
      return isProjectSnapshot(value.project);
    case "cancellation":
      return (
        typeof value.targetRequestId === "string" &&
        (value.outcome === "accepted" ||
          value.outcome === "alreadyFinished" ||
          value.outcome === "notFound")
      );
    case "subscriptionReady":
      return typeof value.streamId === "string";
    default:
      return false;
  }
}

function isDaemonErrorPayload(value: unknown): value is DaemonErrorPayload {
  return (
    isRecord(value) &&
    isErrorCode(value.code) &&
    typeof value.message === "string"
  );
}

function isErrorCode(value: unknown): value is ErrorCode {
  return (
    value === "invalidRequest" ||
    value === "notGitRepository" ||
    value === "projectPathUnavailable" ||
    value === "operationCancelled" ||
    value === "requestInProgress" ||
    value === "serverBusy" ||
    value === "internalError"
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
