import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  DaemonResponseError,
  ProtocolDecodeError,
  type DaemonStreamUpdate,
  type HealthSnapshot
} from "@kesharon/protocol";
import { tauriBridge } from "./bridge";

type ChannelCallback = (data: unknown) => void;

let channelCallback: ChannelCallback | null = null;

vi.mock("@tauri-apps/api/core", () => {
  return {
    invoke: vi.fn(),
    Channel: class MockChannel {
      constructor(onMessage: ChannelCallback) {
        channelCallback = onMessage;
      }
    }
  };
});

import { invoke } from "@tauri-apps/api/core";

const mockedInvoke = vi.mocked(invoke);

describe("tauriBridge", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    channelCallback = null;
  });

  it("invokes daemon_health and decodes HealthSnapshot", async () => {
    mockedInvoke.mockResolvedValueOnce({
      protocolVersion: 1,
      requestId: "health-req-1",
      result: {
        type: "health",
        status: "ready",
        protocolVersion: 1
      },
      error: null
    });

    const health: HealthSnapshot = await tauriBridge.getHealth();
    expect(mockedInvoke).toHaveBeenCalledWith("daemon_health");
    expect(health).toEqual({
      requestId: "health-req-1",
      status: "ready",
      protocolVersion: 1
    });
  });

  it("invokes choose_project_directory and returns path or null", async () => {
    mockedInvoke.mockResolvedValueOnce("D:\\code\\my-project");
    const path = await tauriBridge.chooseProjectDirectory();
    expect(mockedInvoke).toHaveBeenCalledWith("choose_project_directory");
    expect(path).toBe("D:\\code\\my-project");

    mockedInvoke.mockResolvedValueOnce(null);
    const cancelled = await tauriBridge.chooseProjectDirectory();
    expect(cancelled).toBeNull();
  });

  it("invokes daemon_open_project with path and decodes project", async () => {
    mockedInvoke.mockResolvedValueOnce({
      protocolVersion: 1,
      requestId: "open-req-1",
      result: {
        type: "projectOpened",
        project: {
          id: "proj-123",
          displayName: "kesharon",
          canonicalRoot: "D:\\code\\kesharon",
          trusted: true
        }
      },
      error: null
    });

    const result = await tauriBridge.openProject("D:\\code\\kesharon");
    expect(mockedInvoke).toHaveBeenCalledWith("daemon_open_project", {
      path: "D:\\code\\kesharon"
    });
    expect(result).toEqual({
      requestId: "open-req-1",
      project: {
        id: "proj-123",
        displayName: "kesharon",
        canonicalRoot: "D:\\code\\kesharon",
        trusted: true
      }
    });
  });

  it("propagates daemon error on openProject failure", async () => {
    mockedInvoke.mockResolvedValueOnce({
      protocolVersion: 1,
      requestId: "open-req-2",
      result: null,
      error: {
        code: "notGitRepository",
        message: "Selected directory is not a Git worktree"
      }
    });

    await expect(
      tauriBridge.openProject("D:\\invalid\\dir")
    ).rejects.toThrow(
      new DaemonResponseError(
        "notGitRepository",
        "Selected directory is not a Git worktree"
      )
    );
  });

  it("invokes daemon_cancel_request with targetRequestId and decodes result", async () => {
    mockedInvoke.mockResolvedValueOnce({
      protocolVersion: 1,
      requestId: "cancel-req-1",
      result: {
        type: "cancellation",
        targetRequestId: "open-req-1",
        outcome: "accepted"
      },
      error: null
    });

    const result = await tauriBridge.cancelRequest("open-req-1");
    expect(mockedInvoke).toHaveBeenCalledWith("daemon_cancel_request", {
      targetRequestId: "open-req-1"
    });
    expect(result).toEqual({
      requestId: "cancel-req-1",
      targetRequestId: "open-req-1",
      outcome: "accepted"
    });
  });

  it("subscribes to daemon events via Channel and forwards decoded updates", async () => {
    mockedInvoke.mockResolvedValueOnce(undefined);
    const updates: DaemonStreamUpdate[] = [];

    const unsubscribe = await tauriBridge.subscribeDaemonEvents((update) => {
      updates.push(update);
    });

    expect(mockedInvoke).toHaveBeenCalledWith("subscribe_daemon_events", {
      onEvent: expect.any(Object)
    });
    expect(channelCallback).toBeTypeOf("function");

    // Simulate snapshot message
    channelCallback?.({
      type: "message",
      message: {
        messageType: "snapshot",
        snapshot: {
          streamId: "stream-1",
          lastSequence: 2,
          project: {
            id: "proj-1",
            displayName: "kesharon",
            canonicalRoot: "D:\\code\\kesharon",
            trusted: true
          },
          activeOperations: []
        }
      }
    });

    // Simulate reconnecting update
    channelCallback?.({
      type: "reconnecting"
    });

    // Simulate unavailable update
    channelCallback?.({
      type: "unavailable",
      message: "Process terminated"
    });

    expect(updates).toHaveLength(3);
    expect(updates[0]).toEqual({
      type: "message",
      message: {
        messageType: "snapshot",
        snapshot: {
          streamId: "stream-1",
          lastSequence: 2,
          project: {
            id: "proj-1",
            displayName: "kesharon",
            canonicalRoot: "D:\\code\\kesharon",
            trusted: true
          },
          activeOperations: []
        }
      }
    });
    expect(updates[1]).toEqual({ type: "reconnecting" });
    expect(updates[2]).toEqual({
      type: "unavailable",
      message: "Process terminated"
    });

    expect(typeof unsubscribe).toBe("function");
  });
});
