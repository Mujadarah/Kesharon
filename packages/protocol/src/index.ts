export const PROTOCOL_VERSION = 1 as const;

export type HealthStatus = "ready" | "degraded";

export interface HealthSnapshot {
  requestId: string;
  status: HealthStatus;
  protocolVersion: typeof PROTOCOL_VERSION;
}

export class ProtocolDecodeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProtocolDecodeError";
  }
}

export function decodeHealthResponse(value: unknown): HealthSnapshot {
  if (!isRecord(value)) {
    throw new ProtocolDecodeError("Malformed daemon response");
  }

  if (value.protocolVersion !== PROTOCOL_VERSION) {
    const version =
      typeof value.protocolVersion === "number"
        ? String(value.protocolVersion)
        : "unknown";
    throw new ProtocolDecodeError(`Unsupported protocol version ${version}`);
  }

  if (isRecord(value.error) && typeof value.error.message === "string") {
    throw new ProtocolDecodeError(value.error.message);
  }

  if (
    typeof value.requestId !== "string" ||
    !isRecord(value.result) ||
    value.result.type !== "health" ||
    (value.result.status !== "ready" && value.result.status !== "degraded") ||
    value.result.protocolVersion !== PROTOCOL_VERSION
  ) {
    throw new ProtocolDecodeError("Malformed daemon response");
  }

  return {
    requestId: value.requestId,
    status: value.result.status,
    protocolVersion: value.result.protocolVersion
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
