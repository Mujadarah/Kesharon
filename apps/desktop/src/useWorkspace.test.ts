import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  DaemonResponseError,
  type CancellationResult,
  type DaemonStreamUpdate,
  type HealthSnapshot,
  type ProjectOpenResult,
  type ProjectSnapshot,
  type StreamMessage
} from "@kesharon/protocol";
import { useWorkspace } from "./useWorkspace";
import type { DesktopBridge } from "./bridge";

function createMockBridge(overrides: Partial<DesktopBridge> = {}) {
  let streamListener: ((update: DaemonStreamUpdate) => void) | null = null;

  const bridge: DesktopBridge & {
    emitUpdate: (update: DaemonStreamUpdate) => void;
    emitMessage: (message: StreamMessage) => void;
  } = {
    getHealth: vi.fn(async (): Promise<HealthSnapshot> => ({
      requestId: "health-1",
      status: "ready",
      protocolVersion: 1
    })),
    chooseProjectDirectory: vi.fn(async () => "D:\\code\\my-project"),
    openProject: vi.fn(async (path: string): Promise<ProjectOpenResult> => ({
      requestId: "req-open-1",
      project: {
        id: "proj-1",
        displayName: "my-project",
        canonicalRoot: path,
        trusted: true
      }
    })),
    cancelRequest: vi.fn(async (targetRequestId: string): Promise<CancellationResult> => ({
      requestId: "req-cancel-1",
      targetRequestId,
      outcome: "accepted"
    })),
    subscribeDaemonEvents: vi.fn(async (onUpdate) => {
      streamListener = onUpdate;
      return () => {
        streamListener = null;
      };
    }),
    emitUpdate(update: DaemonStreamUpdate) {
      if (streamListener) {
        streamListener(update);
      }
    },
    emitMessage(message: StreamMessage) {
      if (streamListener) {
        streamListener({ type: "message", message });
      }
    },
    ...overrides
  };

  return bridge;
}

describe("useWorkspace", () => {
  it("initializes with checking then ready state upon successful health check", async () => {
    const bridge = createMockBridge();
    const { result } = renderHook(() => useWorkspace(bridge));

    expect(result.current.state.connection.kind).toBe("checking");

    await waitFor(() => {
      expect(result.current.state.connection.kind).toBe("ready");
    });
    expect(bridge.getHealth).toHaveBeenCalled();
    expect(bridge.subscribeDaemonEvents).toHaveBeenCalled();
  });

  it("handles health check failure with unavailable connection state", async () => {
    const bridge = createMockBridge({
      getHealth: vi.fn(async () => {
        throw new Error("IPC pipe broken");
      })
    });
    const { result } = renderHook(() => useWorkspace(bridge));

    await waitFor(() => {
      expect(result.current.state.connection.kind).toBe("unavailable");
    });
  });

  it("reconciles state from authoritative workspace snapshot", async () => {
    const bridge = createMockBridge();
    const { result } = renderHook(() => useWorkspace(bridge));

    await waitFor(() => {
      expect(result.current.state.connection.kind).toBe("ready");
    });

    const project: ProjectSnapshot = {
      id: "p1",
      displayName: "kesharon",
      canonicalRoot: "D:\\code\\kesharon",
      trusted: true
    };

    act(() => {
      bridge.emitMessage({
        messageType: "snapshot",
        snapshot: {
          streamId: "stream-alpha",
          lastSequence: 10,
          project,
          activeOperations: []
        }
      });
    });

    expect(result.current.state.project).toEqual(project);
    expect(result.current.state.streamId).toBe("stream-alpha");
    expect(result.current.state.lastSequence).toBe(10);
    expect(result.current.state.openingOperation).toBeNull();
  });

  it("applies sequenced daemon events correctly", async () => {
    const bridge = createMockBridge();
    const { result } = renderHook(() => useWorkspace(bridge));

    await waitFor(() => {
      expect(result.current.state.connection.kind).toBe("ready");
    });

    // 1. operationStarted
    act(() => {
      bridge.emitMessage({
        messageType: "event",
        event: {
          protocolVersion: 1,
          streamId: "stream-alpha",
          sequence: 1,
          payload: {
            type: "operationStarted",
            requestId: "req-open-99",
            kind: "openProject"
          }
        }
      });
    });

    expect(result.current.state.openingOperation).toEqual({
      requestId: "req-open-99",
      path: "",
      status: "opening"
    });
    expect(result.current.state.lastSequence).toBe(1);

    // 2. projectOpened
    const newProject: ProjectSnapshot = {
      id: "proj-99",
      displayName: "app",
      canonicalRoot: "D:\\code\\app",
      trusted: false
    };

    act(() => {
      bridge.emitMessage({
        messageType: "event",
        event: {
          protocolVersion: 1,
          streamId: "stream-alpha",
          sequence: 2,
          payload: {
            type: "projectOpened",
            requestId: "req-open-99",
            project: newProject
          }
        }
      });
    });

    expect(result.current.state.project).toEqual(newProject);
    expect(result.current.state.openingOperation).toBeNull();
    expect(result.current.state.openingError).toBeNull();
    expect(result.current.state.lastSequence).toBe(2);
  });

  it("handles operationCancelled and operationFailed events", async () => {
    const bridge = createMockBridge();
    const { result } = renderHook(() => useWorkspace(bridge));

    await waitFor(() => {
      expect(result.current.state.connection.kind).toBe("ready");
    });

    // Start operation
    act(() => {
      bridge.emitMessage({
        messageType: "event",
        event: {
          protocolVersion: 1,
          streamId: "s1",
          sequence: 1,
          payload: {
            type: "operationStarted",
            requestId: "req-1",
            kind: "openProject"
          }
        }
      });
    });
    expect(result.current.state.openingOperation).not.toBeNull();

    // Cancel event
    act(() => {
      bridge.emitMessage({
        messageType: "event",
        event: {
          protocolVersion: 1,
          streamId: "s1",
          sequence: 2,
          payload: {
            type: "operationCancelled",
            requestId: "req-1"
          }
        }
      });
    });
    expect(result.current.state.openingOperation).toBeNull();

    // Fail event
    act(() => {
      bridge.emitMessage({
        messageType: "event",
        event: {
          protocolVersion: 1,
          streamId: "s1",
          sequence: 3,
          payload: {
            type: "operationFailed",
            requestId: "req-2",
            code: "notGitRepository",
            message: "Selected directory is not a Git worktree"
          }
        }
      });
    });
    expect(result.current.state.openingError).toBe(
      "Selected directory is not a Git worktree"
    );
  });

  it("handles chooseAndOpen workflow", async () => {
    const bridge = createMockBridge();
    const { result } = renderHook(() => useWorkspace(bridge));

    await waitFor(() => {
      expect(result.current.state.connection.kind).toBe("ready");
    });

    await act(async () => {
      await result.current.chooseAndOpen();
    });

    expect(bridge.chooseProjectDirectory).toHaveBeenCalled();
    expect(bridge.openProject).toHaveBeenCalledWith("D:\\code\\my-project");
    expect(result.current.state.project).toEqual({
      id: "proj-1",
      displayName: "my-project",
      canonicalRoot: "D:\\code\\my-project",
      trusted: true
    });
    expect(result.current.state.openingError).toBeNull();
  });

  it("does not call openProject if user cancels directory picker", async () => {
    const bridge = createMockBridge({
      chooseProjectDirectory: vi.fn(async () => null)
    });
    const { result } = renderHook(() => useWorkspace(bridge));

    await waitFor(() => {
      expect(result.current.state.connection.kind).toBe("ready");
    });

    await act(async () => {
      await result.current.chooseAndOpen();
    });

    expect(bridge.chooseProjectDirectory).toHaveBeenCalled();
    expect(bridge.openProject).not.toHaveBeenCalled();
    expect(result.current.state.openingOperation).toBeNull();
  });

  it("handles error during openProject call", async () => {
    const bridge = createMockBridge({
      openProject: vi.fn(async () => {
        throw new DaemonResponseError(
          "notGitRepository",
          "Selected directory is not a Git worktree"
        );
      })
    });
    const { result } = renderHook(() => useWorkspace(bridge));

    await waitFor(() => {
      expect(result.current.state.connection.kind).toBe("ready");
    });

    await act(async () => {
      await result.current.chooseAndOpen();
    });

    expect(result.current.state.openingOperation).toBeNull();
    expect(result.current.state.openingError).toBe(
      "Selected directory is not a Git worktree"
    );
  });

  it("handles cancelOpening action", async () => {
    const bridge = createMockBridge();
    const { result } = renderHook(() => useWorkspace(bridge));

    await waitFor(() => {
      expect(result.current.state.connection.kind).toBe("ready");
    });

    act(() => {
      bridge.emitMessage({
        messageType: "event",
        event: {
          protocolVersion: 1,
          streamId: "s1",
          sequence: 1,
          payload: {
            type: "operationStarted",
            requestId: "open-op-42",
            kind: "openProject"
          }
        }
      });
    });

    expect(result.current.state.openingOperation?.requestId).toBe("open-op-42");

    await act(async () => {
      await result.current.cancelOpening();
    });

    expect(bridge.cancelRequest).toHaveBeenCalledWith("open-op-42");
  });

  it("updates task goal and clears error", async () => {
    const bridge = createMockBridge();
    const { result } = renderHook(() => useWorkspace(bridge));

    act(() => {
      result.current.setTaskGoal("Fix memory leak in parser");
    });
    expect(result.current.state.taskGoal).toBe("Fix memory leak in parser");

    act(() => {
      bridge.emitMessage({
        messageType: "event",
        event: {
          protocolVersion: 1,
          streamId: "s1",
          sequence: 1,
          payload: {
            type: "operationFailed",
            requestId: "r1",
            code: "internalError",
            message: "Something failed"
          }
        }
      });
    });
    expect(result.current.state.openingError).toBe("Something failed");

    act(() => {
      result.current.clearError();
    });
    expect(result.current.state.openingError).toBeNull();
  });

  it("handles reconnecting and unavailable stream transitions", async () => {
    const bridge = createMockBridge();
    const { result } = renderHook(() => useWorkspace(bridge));

    await waitFor(() => {
      expect(result.current.state.connection.kind).toBe("ready");
    });

    // Reconnecting update
    act(() => {
      bridge.emitUpdate({ type: "reconnecting" });
    });
    expect(result.current.state.connection.kind).toBe("reconnecting");

    // Snapshot recovers to ready
    act(() => {
      bridge.emitMessage({
        messageType: "snapshot",
        snapshot: {
          streamId: "s2",
          lastSequence: 0,
          project: null,
          activeOperations: []
        }
      });
    });
    expect(result.current.state.connection.kind).toBe("ready");

    // Unavailable update
    act(() => {
      bridge.emitUpdate({
        type: "unavailable",
        message: "Daemon crashed"
      });
    });
    expect(result.current.state.connection.kind).toBe("unavailable");
  });
});
