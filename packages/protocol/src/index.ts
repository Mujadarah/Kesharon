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

export interface ResourceBudgetSnapshot {
  maxMemoryBytes: number;
  maxDiskWriteBytes: number;
  maxConcurrentTools: number;
  maxPromptTokens?: number;
  maxCompletionTokens?: number;
  maxCostMicros?: number;
}

export interface TaskSnapshot {
  id: string;
  goal: string;
  state: string;
  executionMode: string;
  budget: ResourceBudgetSnapshot;
}

export interface WorkspaceSnapshot {
  streamId: string;
  lastSequence: number;
  project: ProjectSnapshot | null;
  activeOperations: OperationSnapshot[];
  activeTask: TaskSnapshot | null;
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

export type CancellationOutcome =
  | "accepted"
  | "alreadyFinished"
  | "notFound";

export interface ProjectOpenResult {
  requestId: string;
  project: ProjectSnapshot;
}

export interface CancellationResult {
  requestId: string;
  targetRequestId: string;
  outcome: CancellationOutcome;
}

export interface SubscriptionReadyResult {
  requestId: string;
  streamId: string;
}

export type DaemonStreamUpdate =
  | { type: "message"; message: StreamMessage }
  | { type: "reconnecting" }
  | { type: "unavailable"; message: string };

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
      outcome: CancellationOutcome;
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

export function decodeProjectOpenResponse(value: unknown): ProjectOpenResult {
  const response = decodeDaemonResponse(value);
  const result = requireResult(response);
  if (result.type !== "projectOpened") {
    throw new ProtocolDecodeError("Malformed daemon response");
  }
  return { requestId: response.requestId, project: result.project };
}

export function decodeCancellationResponse(value: unknown): CancellationResult {
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

export function decodeSubscriptionReadyResponse(
  value: unknown
): SubscriptionReadyResult {
  const response = decodeDaemonResponse(value);
  const result = requireResult(response);
  if (result.type !== "subscriptionReady") {
    throw new ProtocolDecodeError("Malformed daemon response");
  }
  return { requestId: response.requestId, streamId: result.streamId };
}

export function decodeDaemonStreamUpdate(value: unknown): DaemonStreamUpdate {
  if (!isRecord(value) || typeof value.type !== "string") {
    throw new ProtocolDecodeError("Malformed daemon stream update");
  }
  if (value.type === "message") {
    return {
      type: "message",
      message: decodeStreamMessage(value.message)
    };
  }
  if (value.type === "reconnecting") {
    return { type: "reconnecting" };
  }
  if (value.type === "unavailable" && typeof value.message === "string") {
    return { type: "unavailable", message: value.message };
  }
  throw new ProtocolDecodeError("Malformed daemon stream update");
}

export function decodeStreamMessage(value: unknown): StreamMessage {
  if (!isRecord(value)) {
    throw new ProtocolDecodeError("Malformed daemon stream message");
  }
  if (value.messageType === "snapshot") {
    return {
      messageType: "snapshot",
      snapshot: decodeWorkspaceSnapshot(value.snapshot)
    };
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

export function decodeResourceBudgetSnapshot(
  value: unknown
): ResourceBudgetSnapshot {
  if (!isResourceBudgetSnapshot(value)) {
    throw new ProtocolDecodeError("Malformed resource budget snapshot");
  }
  return {
    maxMemoryBytes: value.maxMemoryBytes,
    maxDiskWriteBytes: value.maxDiskWriteBytes,
    maxConcurrentTools: value.maxConcurrentTools,
    ...(value.maxPromptTokens !== undefined && {
      maxPromptTokens: value.maxPromptTokens
    }),
    ...(value.maxCompletionTokens !== undefined && {
      maxCompletionTokens: value.maxCompletionTokens
    }),
    ...(value.maxCostMicros !== undefined && {
      maxCostMicros: value.maxCostMicros
    })
  };
}

export function decodeTaskSnapshot(value: unknown): TaskSnapshot {
  if (!isTaskSnapshot(value)) {
    throw new ProtocolDecodeError("Malformed task snapshot");
  }
  return {
    id: value.id,
    goal: value.goal,
    state: value.state,
    executionMode: value.executionMode,
    budget: decodeResourceBudgetSnapshot(value.budget)
  };
}

export function decodeWorkspaceSnapshot(value: unknown): WorkspaceSnapshot {
  const candidate =
    isRecord(value) && value.messageType === "snapshot"
      ? value.snapshot
      : value;

  if (!isWorkspaceSnapshotCandidate(candidate)) {
    throw new ProtocolDecodeError("Malformed workspace snapshot");
  }

  return {
    streamId: candidate.streamId,
    lastSequence: candidate.lastSequence,
    project: candidate.project ? { ...candidate.project } : null,
    activeOperations: candidate.activeOperations.map((operation) => ({
      requestId: operation.requestId,
      kind: operation.kind,
      status: operation.status
    })),
    activeTask:
      candidate.activeTask !== undefined && candidate.activeTask !== null
        ? decodeTaskSnapshot(candidate.activeTask)
        : null
  };
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

function isResourceBudgetSnapshot(
  value: unknown
): value is ResourceBudgetSnapshot {
  if (!isRecord(value)) {
    return false;
  }
  if (
    typeof value.maxMemoryBytes !== "number" ||
    !Number.isFinite(value.maxMemoryBytes) ||
    value.maxMemoryBytes < 0 ||
    typeof value.maxDiskWriteBytes !== "number" ||
    !Number.isFinite(value.maxDiskWriteBytes) ||
    value.maxDiskWriteBytes < 0 ||
    typeof value.maxConcurrentTools !== "number" ||
    !Number.isInteger(value.maxConcurrentTools) ||
    value.maxConcurrentTools < 0
  ) {
    return false;
  }
  if (
    value.maxPromptTokens !== undefined &&
    (typeof value.maxPromptTokens !== "number" ||
      !Number.isFinite(value.maxPromptTokens) ||
      value.maxPromptTokens < 0)
  ) {
    return false;
  }
  if (
    value.maxCompletionTokens !== undefined &&
    (typeof value.maxCompletionTokens !== "number" ||
      !Number.isFinite(value.maxCompletionTokens) ||
      value.maxCompletionTokens < 0)
  ) {
    return false;
  }
  if (
    value.maxCostMicros !== undefined &&
    (typeof value.maxCostMicros !== "number" ||
      !Number.isFinite(value.maxCostMicros) ||
      value.maxCostMicros < 0)
  ) {
    return false;
  }
  return true;
}

function isTaskSnapshot(value: unknown): value is TaskSnapshot {
  return (
    isRecord(value) &&
    typeof value.id === "string" &&
    typeof value.goal === "string" &&
    typeof value.state === "string" &&
    typeof value.executionMode === "string" &&
    isResourceBudgetSnapshot(value.budget)
  );
}

interface WorkspaceSnapshotCandidate {
  streamId: string;
  lastSequence: number;
  project: ProjectSnapshot | null;
  activeOperations: OperationSnapshot[];
  activeTask?: unknown;
}

function isWorkspaceSnapshotCandidate(
  value: unknown
): value is WorkspaceSnapshotCandidate {
  if (
    !isRecord(value) ||
    typeof value.streamId !== "string" ||
    !Number.isSafeInteger(value.lastSequence) ||
    (value.lastSequence as number) < 0 ||
    (value.project !== null && !isProjectSnapshot(value.project)) ||
    !Array.isArray(value.activeOperations) ||
    !value.activeOperations.every(
      (operation) =>
        isRecord(operation) &&
        typeof operation.requestId === "string" &&
        operation.kind === "openProject" &&
        operation.status === "running"
    )
  ) {
    return false;
  }
  if (
    value.activeTask !== undefined &&
    value.activeTask !== null &&
    !isTaskSnapshot(value.activeTask)
  ) {
    return false;
  }
  return true;
}

function isWorkspaceSnapshot(value: unknown): value is WorkspaceSnapshot {
  return (
    isWorkspaceSnapshotCandidate(value) &&
    (value.activeTask === null || isTaskSnapshot(value.activeTask))
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
