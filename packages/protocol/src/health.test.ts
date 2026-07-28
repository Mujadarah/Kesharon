import { describe, expect, it } from "vitest";

import { ProtocolDecodeError, decodeHealthResponse } from "./index";

describe("decodeHealthResponse", () => {
  it("accepts the current ready response", () => {
    expect(
      decodeHealthResponse({
        protocolVersion: 1,
        requestId: "request-1",
        result: {
          type: "health",
          status: "ready",
          protocolVersion: 1
        },
        error: null
      })
    ).toEqual({
      requestId: "request-1",
      status: "ready",
      protocolVersion: 1
    });
  });

  it("rejects an unsupported protocol version", () => {
    expect(() =>
      decodeHealthResponse({
        protocolVersion: 2,
        requestId: "request-2",
        result: {
          type: "health",
          status: "ready",
          protocolVersion: 2
        },
        error: null
      })
    ).toThrow(new ProtocolDecodeError("Unsupported protocol version 2"));
  });

  it("rejects malformed or error responses", () => {
    expect(() => decodeHealthResponse(null)).toThrow(ProtocolDecodeError);
    expect(() =>
      decodeHealthResponse({
        protocolVersion: 1,
        requestId: "request-3",
        result: null,
        error: {
          code: "daemon_unavailable",
          message: "The daemon is unavailable"
        }
      })
    ).toThrow(new ProtocolDecodeError("The daemon is unavailable"));
  });
});
