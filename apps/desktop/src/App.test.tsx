import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  DaemonResponseError,
  type CancellationResult,
  type DaemonStreamUpdate,
  type HealthSnapshot,
  type ProjectOpenResult,
  type StreamMessage,
  type TaskSnapshot
} from "@kesharon/protocol";
import { App } from "./App";
import type { DesktopBridge } from "./bridge";

function createMockBridge(overrides: Partial<DesktopBridge> = {}) {
  let streamListener: ((update: DaemonStreamUpdate) => void) | null = null;

  const bridge: DesktopBridge & {
    emitUpdate: (update: DaemonStreamUpdate) => void;
    emitMessage: (message: StreamMessage) => void;
  } = {
    getHealth: vi.fn(async (): Promise<HealthSnapshot> => ({
      requestId: "request-ui-1",
      status: "ready",
      protocolVersion: 1
    })),
    chooseProjectDirectory: vi.fn(async () => "D:\\code\\kesharon-agent"),
    openProject: vi.fn(async (path: string): Promise<ProjectOpenResult> => ({
      requestId: "open-req-1",
      project: {
        id: "proj-101",
        displayName: "kesharon-agent",
        canonicalRoot: path,
        trusted: true
      }
    })),
    cancelRequest: vi.fn(async (targetRequestId: string): Promise<CancellationResult> => ({
      requestId: "cancel-req-1",
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

describe("App Component", () => {
  afterEach(() => {
    cleanup();
  });

  it("shows the workspace layout and daemon status on ready", async () => {
    const bridge = createMockBridge();
    render(<App bridge={bridge} />);

    expect(
      screen.getByRole("heading", { name: "Plan before execution" })
    ).toBeInTheDocument();
    expect(
      screen.getByRole("navigation", { name: "Workspace navigation" })
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Plan and assistant" })
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Files, diff, and review" })
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Resource monitor" })
    ).toBeInTheDocument();

    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();
    expect(screen.getByText("Protocol v1")).toBeInTheDocument();
  });

  it("turns a health failure into a directed recovery state", async () => {
    const bridge = createMockBridge({
      getHealth: vi.fn(async () => {
        throw new Error("Named pipe is unavailable");
      })
    });

    render(<App bridge={bridge} />);

    expect(await screen.findByText("Daemon unavailable")).toBeInTheDocument();
    expect(
      screen.getByText("Restart Kesharon, then check the connection again.")
    ).toBeInTheDocument();
  });

  it("triggers open project when clicking 'Open repository' button", async () => {
    const bridge = createMockBridge();
    render(<App bridge={bridge} />);

    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

    const openButton = screen.getByRole("button", { name: "Open repository" });
    await act(async () => {
      fireEvent.click(openButton);
    });

    expect(bridge.chooseProjectDirectory).toHaveBeenCalled();
    expect(bridge.openProject).toHaveBeenCalledWith("D:\\code\\kesharon-agent");

    // Opened repo details are visible
    expect(await screen.findByText("kesharon-agent")).toBeInTheDocument();
    expect(screen.getByText("D:\\code\\kesharon-agent")).toBeInTheDocument();
    expect(screen.getByText("Trusted")).toBeInTheDocument();
  });

  it("triggers open project when clicking '+' (Add project) button", async () => {
    const bridge = createMockBridge();
    render(<App bridge={bridge} />);

    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

    const addButton = screen.getByRole("button", { name: "Add project" });
    await act(async () => {
      fireEvent.click(addButton);
    });

    expect(bridge.chooseProjectDirectory).toHaveBeenCalled();
    expect(bridge.openProject).toHaveBeenCalledWith("D:\\code\\kesharon-agent");
  });

  it("displays real-time opening progress card with responsive Cancel action", async () => {
    let resolveOpen: (result: ProjectOpenResult) => void = () => {};
    const openPromise = new Promise<ProjectOpenResult>((res) => {
      resolveOpen = res;
    });

    const bridge = createMockBridge({
      openProject: vi.fn(() => openPromise)
    });

    render(<App bridge={bridge} />);
    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

    const openButton = screen.getByRole("button", { name: "Open repository" });
    act(() => {
      fireEvent.click(openButton);
    });

    // Opening progress card should appear
    expect(await screen.findByText("Opening repository...")).toBeInTheDocument();
    const cancelButton = screen.getByRole("button", { name: "Cancel opening" });
    expect(cancelButton).toBeInTheDocument();
    expect(cancelButton).toBeEnabled();

    // Click cancel
    await act(async () => {
      fireEvent.click(cancelButton);
    });

    expect(bridge.cancelRequest).toHaveBeenCalled();

    // Clean up promise to avoid unhandled hangs
    act(() => {
      resolveOpen({
        requestId: "open-req-1",
        project: {
          id: "p1",
          displayName: "kesharon",
          canonicalRoot: "D:\\code\\kesharon",
          trusted: true
        }
      });
    });
  });

  it("renders opened repository card with untrusted badge if project is untrusted", async () => {
    const bridge = createMockBridge();
    render(<App bridge={bridge} />);
    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

    act(() => {
      bridge.emitMessage({
        messageType: "snapshot",
        snapshot: {
          streamId: "s-1",
          lastSequence: 1,
          project: {
            id: "untrusted-proj",
            displayName: "external-repo",
            canonicalRoot: "D:\\external\\repo",
            trusted: false
          },
          activeOperations: [],
          activeTask: null
        }
      });
    });

    expect(screen.getByText("external-repo")).toBeInTheDocument();
    expect(screen.getByText("D:\\external\\repo")).toBeInTheDocument();
    expect(screen.getByText("Untrusted (Sandbox)")).toBeInTheDocument();
  });

  it("enables task composer textarea when project is open; Prepare plan button remains disabled", async () => {
    const bridge = createMockBridge();
    render(<App bridge={bridge} />);
    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

    const textarea = screen.getByPlaceholderText(
      "What should change in this repository?"
    ) as HTMLTextAreaElement;
    const prepareButton = screen.getByRole("button", { name: "Prepare plan" });

    // Initially disabled
    expect(textarea).toBeDisabled();
    expect(prepareButton).toBeDisabled();
    expect(
      screen.getByText("Open a repository to enable planning")
    ).toBeInTheDocument();

    // Open project via snapshot
    act(() => {
      bridge.emitMessage({
        messageType: "snapshot",
        snapshot: {
          streamId: "s-1",
          lastSequence: 1,
          project: {
            id: "proj-1",
            displayName: "my-app",
            canonicalRoot: "D:\\code\\my-app",
            trusted: true
          },
          activeOperations: [],
          activeTask: null
        }
      });
    });

    // Textarea is enabled
    expect(textarea).not.toBeDisabled();
    // Prepare plan button MUST remain disabled in Phase 2
    expect(prepareButton).toBeDisabled();
    expect(
      screen.getByText("Planning available · execution disabled in Phase 2")
    ).toBeInTheDocument();

    // User can type into task composer textarea
    fireEvent.change(textarea, {
      target: { value: "Refactor auth middleware to use JWT" }
    });
    expect(textarea.value).toBe("Refactor auth middleware to use JWT");
  });

  it("renders error banner when project opening fails and allows dismissing it", async () => {
    const bridge = createMockBridge({
      openProject: vi.fn(async () => {
        throw new DaemonResponseError(
          "notGitRepository",
          "Selected directory is not a Git worktree"
        );
      })
    });

    render(<App bridge={bridge} />);
    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

    const openButton = screen.getByRole("button", { name: "Open repository" });
    await act(async () => {
      fireEvent.click(openButton);
    });

    expect(
      await screen.findByText("Selected directory is not a Git worktree")
    ).toBeInTheDocument();

    const dismissButton = screen.getByRole("button", { name: "Dismiss error" });
    act(() => {
      fireEvent.click(dismissButton);
    });

    expect(
      screen.queryByText("Selected directory is not a Git worktree")
    ).not.toBeInTheDocument();
  });

  it("handles reconnect transitions without crash or remount errors", async () => {
    const bridge = createMockBridge();
    render(<App bridge={bridge} />);
    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

    // Stream signals reconnecting
    act(() => {
      bridge.emitUpdate({ type: "reconnecting" });
    });
    expect(
      await screen.findByText("Reconnecting to daemon")
    ).toBeInTheDocument();

    // Stream recovers with snapshot
    act(() => {
      bridge.emitMessage({
        messageType: "snapshot",
        snapshot: {
          streamId: "s-reconnect",
          lastSequence: 5,
          project: {
            id: "proj-1",
            displayName: "restored-project",
            canonicalRoot: "D:\\code\\restored",
            trusted: true
          },
          activeOperations: [],
          activeTask: null
        }
      });
    });

    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();
    expect(screen.getByText("restored-project")).toBeInTheDocument();
  });

  it("renders [Mode: PLAN] indicator badge and updates UI with active task details and resource ceilings", async () => {
    const bridge = createMockBridge();
    render(<App bridge={bridge} />);
    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

    const modeBadge = screen.getByTestId("execution-mode-badge");
    expect(modeBadge).toBeInTheDocument();
    expect(modeBadge).toHaveTextContent("[Mode: PLAN]");

    // Default resource ceilings when no active task
    expect(screen.getByText("0 / 2")).toBeInTheDocument();

    const activeTask: TaskSnapshot = {
      id: "task-test-42",
      goal: "Implement token counter and telemetry",
      state: "planning",
      executionMode: "plan",
      budget: {
        maxMemoryBytes: 512 * 1024 * 1024,
        maxDiskWriteBytes: 128 * 1024 * 1024,
        maxConcurrentTools: 4,
        maxPromptTokens: 40000,
        maxCompletionTokens: 8000
      }
    };

    act(() => {
      bridge.emitMessage({
        messageType: "snapshot",
        snapshot: {
          streamId: "s-task-1",
          lastSequence: 1,
          project: null,
          activeOperations: [],
          activeTask
        }
      });
    });

    // Active task goal and status
    const activeTaskBanner = screen.getByTestId("active-task-info");
    expect(activeTaskBanner).toBeInTheDocument();
    expect(screen.getByTestId("active-task-status")).toHaveTextContent("planning");
    expect(within(activeTaskBanner).getByText("Implement token counter and telemetry")).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toHaveValue("Implement token counter and telemetry");

    // ResourcePanel ceilings
    expect(screen.getByText("512 MB")).toBeInTheDocument();
    expect(screen.getByText("128 MB")).toBeInTheDocument();
    expect(screen.getByText("0 / 4")).toBeInTheDocument();
    expect(screen.getByText("40,000 prompt / 8,000 completion")).toBeInTheDocument();
  });

  it("updates mode badge to autonomous, omits tokens metric when undefined, and reverts on clear", async () => {
    const bridge = createMockBridge();
    render(<App bridge={bridge} />);
    expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

    const modeBadge = screen.getByTestId("execution-mode-badge");
    expect(modeBadge).toHaveTextContent("[Mode: PLAN]");

    const autonomousTask: TaskSnapshot = {
      id: "task-auto-1",
      goal: "Run autonomous security scan",
      state: "running",
      executionMode: "autonomous",
      budget: {
        maxMemoryBytes: 256 * 1024 * 1024,
        maxDiskWriteBytes: 32 * 1024 * 1024,
        maxConcurrentTools: 2
      }
    };

    act(() => {
      bridge.emitMessage({
        messageType: "snapshot",
        snapshot: {
          streamId: "s-auto",
          lastSequence: 2,
          project: null,
          activeOperations: [],
          activeTask: autonomousTask
        }
      });
    });

    // Badge shows AUTONOMOUS
    expect(modeBadge).toHaveTextContent("[Mode: AUTONOMOUS]");

    // Active task info shows running status
    expect(screen.getByTestId("active-task-status")).toHaveTextContent("running");
    expect(within(screen.getByTestId("active-task-info")).getByText("Run autonomous security scan")).toBeInTheDocument();

    // Ceilings formatted properly
    expect(screen.getByText("256 MB")).toBeInTheDocument();
    expect(screen.getByText("32 MB")).toBeInTheDocument();
    expect(screen.getByText("0 / 2")).toBeInTheDocument();
    // Tokens metric should not be present
    expect(screen.queryByText("Tokens")).not.toBeInTheDocument();

    // Clear active task via snapshot
    act(() => {
      bridge.emitMessage({
        messageType: "snapshot",
        snapshot: {
          streamId: "s-auto",
          lastSequence: 3,
          project: null,
          activeOperations: [],
          activeTask: null
        }
      });
    });

    // Reverts to default mode badge and removes active task banner
    expect(modeBadge).toHaveTextContent("[Mode: PLAN]");
    expect(screen.queryByTestId("active-task-info")).not.toBeInTheDocument();
  });
});

