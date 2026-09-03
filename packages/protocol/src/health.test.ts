import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import {
  DaemonResponseError,
  ProtocolDecodeError,
  decodeCancellationResponse,
  decodeDaemonResponse,
  decodeDaemonStreamUpdate,
  decodeHealthResponse,
  decodeProjectOpenResponse,
  decodeStreamMessage
} from "./index";

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
          code: "internalError",
          message: "The daemon is unavailable"
        }
      })
    ).toThrow(
      new DaemonResponseError("internalError", "The daemon is unavailable")
    );
  });
});

describe("session protocol", () => {
  const fixtures = JSON.parse(
    readFileSync(
      fileURLToPath(
        new URL("../fixtures/session-messages.json", import.meta.url)
      ),
      "utf8"
    )
  ) as Record<string, unknown>;

  it("decodes the shared workspace snapshot fixture", () => {
    expect(decodeStreamMessage(fixtures.snapshotMessage)).toMatchObject({
      messageType: "snapshot",
      snapshot: {
        streamId: "stream-1",
        lastSequence: 4,
        project: { displayName: "Kesharon", trusted: false },
        activeOperations: [
          { requestId: "request-open-2", status: "running" }
        ]
      }
    });
  });

  it("decodes the shared sequenced event fixture", () => {
    expect(decodeStreamMessage(fixtures.eventMessage)).toMatchObject({
      messageType: "event",
      event: {
        streamId: "stream-1",
        sequence: 5,
        payload: { type: "projectOpened", requestId: "request-open-1" }
      }
    });
  });

  it("decodes every shared terminal and lifecycle event", () => {
    for (const name of [
      "operationStartedMessage",
      "operationCancelledMessage",
      "failureEventMessage"
    ]) {
      expect(decodeStreamMessage(fixtures[name])).toMatchObject({
        messageType: "event",
        event: { streamId: "stream-1" }
      });
    }
  });

  it("rejects malformed event sequences", () => {
    expect(() =>
      decodeStreamMessage({
        messageType: "event",
        event: {
          protocolVersion: 1,
          streamId: "stream-1",
          sequence: 0,
          payload: {
            type: "operationCancelled",
            requestId: "request-open-1"
          }
        }
      })
    ).toThrow(ProtocolDecodeError);
  });

  it("decodes every shared response variant without trusting unknown", () => {
    expect(decodeDaemonResponse(fixtures.healthResponse)).toMatchObject({
      requestId: "request-health-1",
      result: { type: "health", status: "ready" },
      error: null
    });
    expect(decodeProjectOpenResponse(fixtures.projectOpenedResponse)).toEqual({
      requestId: "request-open-1",
      project: {
        id: "project-1",
        displayName: "Kesharon",
        canonicalRoot: "D:\\code\\kesharon",
        trusted: false
      }
    });
    expect(decodeCancellationResponse(fixtures.cancellationResponse)).toEqual({
      requestId: "request-cancel-1",
      targetRequestId: "request-open-1",
      outcome: "accepted"
    });
  });

  it("preserves structured daemon error codes", () => {
    expect(() => decodeProjectOpenResponse(fixtures.failureResponse)).toThrow(
      new DaemonResponseError(
        "notGitRepository",
        "Selected directory is not a Git worktree"
      )
    );
  });

  it("rejects unknown error codes and malformed response payloads", () => {
    expect(() =>
      decodeDaemonResponse({
        protocolVersion: 1,
        requestId: "bad-error",
        result: null,
        error: { code: "madeUp", message: "no" }
      })
    ).toThrow(ProtocolDecodeError);
    expect(() =>
      decodeDaemonResponse({
        protocolVersion: 1,
        requestId: "bad-shape",
        result: { type: "cancellation", outcome: "accepted" },
        error: null
      })
    ).toThrow(ProtocolDecodeError);
  });

  it("rejects unknown failure-event error codes", () => {
    expect(() =>
      decodeStreamMessage({
        messageType: "event",
        event: {
          protocolVersion: 1,
          streamId: "stream-1",
          sequence: 7,
          payload: {
            type: "operationFailed",
            requestId: "request-open-1",
            code: "madeUp",
            message: "no"
          }
        }
      })
    ).toThrow(ProtocolDecodeError);
  });

  it("decodes valid daemon stream updates", () => {
    expect(
      decodeDaemonStreamUpdate({
        type: "message",
        message: fixtures.snapshotMessage
      })
    ).toEqual({
      type: "message",
      message: fixtures.snapshotMessage
    });

    expect(
      decodeDaemonStreamUpdate({
        type: "reconnecting"
      })
    ).toEqual({
      type: "reconnecting"
    });

    expect(
      decodeDaemonStreamUpdate({
        type: "unavailable",
        message: "Daemon process terminated unexpectedly"
      })
    ).toEqual({
      type: "unavailable",
      message: "Daemon process terminated unexpectedly"
    });
  });

  it("rejects invalid daemon stream updates", () => {
    expect(() => decodeDaemonStreamUpdate(null)).toThrow(ProtocolDecodeError);
    expect(() => decodeDaemonStreamUpdate({ type: "unknown" })).toThrow(
      ProtocolDecodeError
    );
    expect(() =>
      decodeDaemonStreamUpdate({ type: "message", message: { invalid: true } })
    ).toThrow(ProtocolDecodeError);
    expect(() => decodeDaemonStreamUpdate({ type: "unavailable" })).toThrow(
      ProtocolDecodeError
    );
  });
});
