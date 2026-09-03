import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  DaemonResponseError,
  type CancellationResult,
  type DaemonStreamUpdate,
  type HealthSnapshot,
  type ProjectOpenResult,
  type StreamMessage
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
      requestId: "req-health-init",
      status: "ready",
      protocolVersion: 1
    })),
    chooseProjectDirectory: vi.fn(async () => "D:\\code\\repo-primary"),
    openProject: vi.fn(async (path: string): Promise<ProjectOpenResult> => ({
      requestId: "req-open-mock",
      project: {
        id: "proj-primary",
        displayName: "repo-primary",
        canonicalRoot: path,
        trusted: true
      }
    })),
    cancelRequest: vi.fn(async (targetRequestId: string): Promise<CancellationResult> => ({
      requestId: "req-cancel-mock",
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

describe("Empirical Challenge Suite: UI Interactions & State Transitions", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  describe("1. State Transition: Empty State -> Picker Dismissal -> Non-Git Error -> Dismiss", () => {
    it("executes the full cycle cleanly without state corruption or stuck UI elements", async () => {
      let pickerResult: string | null = null;
      let openError: Error | null = null;

      const bridge = createMockBridge({
        chooseProjectDirectory: vi.fn(async () => pickerResult),
        openProject: vi.fn(async (path: string) => {
          if (openError) {
            throw openError;
          }
          return {
            requestId: "req-open-1",
            project: {
              id: "proj-1",
              displayName: "valid-project",
              canonicalRoot: path,
              trusted: true
            }
          };
        })
      });

      render(<App bridge={bridge} />);
      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

      // Step 1: Initial Empty State
      expect(screen.getByText("No repository open")).toBeInTheDocument();
      expect(
        screen.getByText("Choose a Git repository to begin a reviewed task.")
      ).toBeInTheDocument();
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      expect(screen.queryByRole("status")).not.toBeInTheDocument();

      // Step 2: User clicks "Open repository", but cancels the native folder picker (returns null)
      pickerResult = null;
      const openRepoBtn = screen.getByRole("button", { name: "Open repository" });
      await act(async () => {
        fireEvent.click(openRepoBtn);
      });

      expect(bridge.chooseProjectDirectory).toHaveBeenCalledTimes(1);
      expect(bridge.openProject).not.toHaveBeenCalled();
      expect(screen.getByText("No repository open")).toBeInTheDocument();
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      expect(screen.queryByRole("status")).not.toBeInTheDocument();

      // Step 3: User clicks "+" (Add project) button and picks a directory that is NOT a Git repo
      pickerResult = "D:\\Users\\Guest\\Downloads";
      openError = new DaemonResponseError(
        "notGitRepository",
        "Directory 'D:\\Users\\Guest\\Downloads' is not a Git worktree"
      );

      const addBtn = screen.getByRole("button", { name: "Add project" });
      await act(async () => {
        fireEvent.click(addBtn);
      });

      expect(bridge.chooseProjectDirectory).toHaveBeenCalledTimes(2);
      expect(bridge.openProject).toHaveBeenCalledWith("D:\\Users\\Guest\\Downloads");

      // Verify Error Banner
      const errorAlert = await screen.findByRole("alert");
      expect(errorAlert).toBeInTheDocument();
      expect(
        screen.getByText("Directory 'D:\\Users\\Guest\\Downloads' is not a Git worktree")
      ).toBeInTheDocument();
      // Empty state persists under error
      expect(screen.getByText("No repository open")).toBeInTheDocument();
      expect(screen.queryByRole("status")).not.toBeInTheDocument();

      // Step 4: User clicks "Dismiss" on the error banner
      const dismissBtn = screen.getByRole("button", { name: "Dismiss error" });
      act(() => {
        fireEvent.click(dismissBtn);
      });

      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      expect(
        screen.queryByText("Directory 'D:\\Users\\Guest\\Downloads' is not a Git worktree")
      ).not.toBeInTheDocument();
      expect(screen.getByText("No repository open")).toBeInTheDocument();
    });
  });

  describe("2. Interactive Cancellation Under Simulated Slow Backend", () => {
    it("handles in-flight opening, responsive cancel trigger, cancelling state, and clean stream cancellation", async () => {
      let resolveCancel: (res: CancellationResult) => void = () => {};
      let rejectOpenProject: (err: Error) => void = () => {};
      const pendingOpen = new Promise<ProjectOpenResult>((_resolve, reject) => {
        rejectOpenProject = reject;
      });

      const pendingCancel = new Promise<CancellationResult>((resolve) => {
        resolveCancel = resolve;
      });

      const bridge = createMockBridge({
        chooseProjectDirectory: vi.fn(async () => "D:\\work\\huge-repo"),
        openProject: vi.fn(() => pendingOpen),
        cancelRequest: vi.fn(() => pendingCancel)
      });

      render(<App bridge={bridge} />);
      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

      const openButton = screen.getByRole("button", { name: "Open repository" });
      act(() => {
        fireEvent.click(openButton);
      });

      // 1. Opening card appears
      const openingStatus = await screen.findByRole("status");
      expect(openingStatus).toBeInTheDocument();
      expect(screen.getByText("Opening repository...")).toBeInTheDocument();
      expect(screen.getByText("D:\\work\\huge-repo")).toBeInTheDocument();

      // Plus button must be disabled during active opening
      const addBtn = screen.getByRole("button", { name: "Add project" });
      expect(addBtn).toBeDisabled();

      // 2. Cancel button is present and enabled
      const cancelBtn = screen.getByRole("button", { name: "Cancel opening" });
      expect(cancelBtn).toBeInTheDocument();
      expect(cancelBtn).toBeEnabled();
      expect(cancelBtn).toHaveTextContent("Cancel");

      // 3. User clicks Cancel
      act(() => {
        fireEvent.click(cancelBtn);
      });

      // Verify cancelRequest was called with the client request ID
      expect(bridge.cancelRequest).toHaveBeenCalledTimes(1);
      const passedRequestId = (bridge.cancelRequest as unknown as { mock: { calls: string[][] } })
        .mock.calls[0]?.[0] ?? "";
      expect(passedRequestId).toMatch(/^req-client-open-/);

      // Button enters cancelling state and disables
      expect(cancelBtn).toHaveTextContent("Cancelling...");
      expect(cancelBtn).toBeDisabled();

      // 4. Resolve the cancelRequest promise
      await act(async () => {
        resolveCancel({
          requestId: "cancel-ack-1",
          targetRequestId: passedRequestId,
          outcome: "accepted"
        });
      });

      // 5. Authoritative daemon event emits operationCancelled
      act(() => {
        bridge.emitMessage({
          messageType: "event",
          event: {
            protocolVersion: 1,
            streamId: "stream-test-1",
            sequence: 1,
            payload: {
              type: "operationCancelled",
              requestId: passedRequestId
            }
          }
        });
      });

      // Reject the pending open with operationCancelled error (simulating daemon abort)
      act(() => {
        rejectOpenProject(
          new DaemonResponseError("operationCancelled", "Open operation was cancelled")
        );
      });

      // 6. UI smoothly transitions out of opening card back to empty state
      await waitFor(() => {
        expect(screen.queryByRole("status")).not.toBeInTheDocument();
      });
      expect(screen.getByText("No repository open")).toBeInTheDocument();
      expect(addBtn).toBeEnabled();
    });

    it("survives unexpected rejection from cancelRequest without crashing the UI", async () => {
      const consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

      let resolveOpen: (res: ProjectOpenResult) => void = () => {};
      const pendingOpen = new Promise<ProjectOpenResult>((res) => {
        resolveOpen = res;
      });

      const bridge = createMockBridge({
        openProject: vi.fn(() => pendingOpen),
        cancelRequest: vi.fn(async () => {
          throw new Error("IPC socket transport failure during cancel");
        })
      });

      render(<App bridge={bridge} />);
      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

      const openButton = screen.getByRole("button", { name: "Open repository" });
      act(() => {
        fireEvent.click(openButton);
      });

      expect(await screen.findByRole("status")).toBeInTheDocument();
      const cancelBtn = screen.getByRole("button", { name: "Cancel opening" });

      await act(async () => {
        fireEvent.click(cancelBtn);
      });

      expect(bridge.cancelRequest).toHaveBeenCalled();
      expect(consoleErrorSpy).toHaveBeenCalledWith(
        "Failed to cancel opening request",
        expect.any(Error)
      );

      // Clean up open promise
      act(() => {
        resolveOpen({
          requestId: "req-clean",
          project: {
            id: "proj-clean",
            displayName: "clean",
            canonicalRoot: "D:\\clean",
            trusted: true
          }
        });
      });
      consoleErrorSpy.mockRestore();
    });
  });

  describe("3. Task Composer Controlled Input & Mutation Guarding", () => {
    it("strictly disables textarea and Prepare Plan button when no project is open", async () => {
      const bridge = createMockBridge();
      render(<App bridge={bridge} />);
      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

      const textarea = screen.getByPlaceholderText(
        "What should change in this repository?"
      ) as HTMLTextAreaElement;
      const preparePlanBtn = screen.getByRole("button", { name: "Prepare plan" });
      const hint = screen.getByText("Open a repository to enable planning");

      expect(textarea).toBeDisabled();
      expect(textarea.value).toBe("");
      expect(preparePlanBtn).toBeDisabled();
      expect(hint).toBeInTheDocument();

      // Attempting user event on disabled textarea does not update value
      fireEvent.change(textarea, { target: { value: "Malicious mutation attempt" } });
      expect(textarea.value).toBe("");
    });

    it("enables textarea upon project open, preserves controlled value, and KEEPS Prepare Plan button strictly disabled", async () => {
      const bridge = createMockBridge();
      render(<App bridge={bridge} />);
      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

      const textarea = screen.getByPlaceholderText(
        "What should change in this repository?"
      ) as HTMLTextAreaElement;
      const preparePlanBtn = screen.getByRole("button", { name: "Prepare plan" });

      // Open a project via stream snapshot
      act(() => {
        bridge.emitMessage({
          messageType: "snapshot",
          snapshot: {
            streamId: "stream-main",
            lastSequence: 10,
            project: {
              id: "proj-kesharon",
              displayName: "kesharon-agent",
              canonicalRoot: "D:\\code\\kesharon-agent",
              trusted: true
            },
            activeOperations: []
          }
        });
      });

      // Textarea is enabled; helper text indicates execution is disabled in Phase 2
      expect(textarea).toBeEnabled();
      expect(
        screen.getByText("Planning available · execution disabled in Phase 2")
      ).toBeInTheDocument();

      // Crucial Gate: Prepare Plan button MUST BE DISABLED in Phase 2
      expect(preparePlanBtn).toBeDisabled();

      // Test 1: Simple single-line text input
      fireEvent.change(textarea, {
        target: { value: "Refactor error handling in daemon" }
      });
      expect(textarea.value).toBe("Refactor error handling in daemon");
      expect(preparePlanBtn).toBeDisabled();

      // Test 2: Multiline code instructions with special characters and emojis
      const complexGoal = `Implement authentication middleware:\n1. Check bearer token\n2. Verify JWT signature\n3. Return 401 if invalid\n🚀 Code: \`fn authenticate(req: &Request) -> Result<User, AuthError>\``;
      fireEvent.change(textarea, {
        target: { value: complexGoal }
      });
      expect(textarea.value).toBe(complexGoal);
      expect(preparePlanBtn).toBeDisabled();

      // Test 3: Large input stress (50,000 chars)
      const largeInput = "A".repeat(50000);
      fireEvent.change(textarea, {
        target: { value: largeInput }
      });
      expect(textarea.value).toBe(largeInput);
      expect(preparePlanBtn).toBeDisabled();

      // Test 4: Clearing the input
      fireEvent.change(textarea, {
        target: { value: "" }
      });
      expect(textarea.value).toBe("");
      expect(preparePlanBtn).toBeDisabled();
    });

    it("disables textarea if project is unloaded while retaining draft state for when reconnected", async () => {
      const bridge = createMockBridge();
      render(<App bridge={bridge} />);
      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

      const textarea = screen.getByPlaceholderText(
        "What should change in this repository?"
      ) as HTMLTextAreaElement;

      // 1. Open project
      act(() => {
        bridge.emitMessage({
          messageType: "snapshot",
          snapshot: {
            streamId: "stream-s1",
            lastSequence: 1,
            project: {
              id: "proj-1",
              displayName: "repo-1",
              canonicalRoot: "D:\\code\\repo-1",
              trusted: true
            },
            activeOperations: []
          }
        });
      });

      expect(textarea).toBeEnabled();

      // 2. Draft task goal
      fireEvent.change(textarea, {
        target: { value: "Draft task in progress" }
      });
      expect(textarea.value).toBe("Draft task in progress");

      // 3. Daemon unloads project (e.g. workspace cleared)
      act(() => {
        bridge.emitMessage({
          messageType: "snapshot",
          snapshot: {
            streamId: "stream-s1",
            lastSequence: 2,
            project: null,
            activeOperations: []
          }
        });
      });

      // Textarea is disabled, but existing value remains in state
      expect(textarea).toBeDisabled();
      expect(textarea.value).toBe("Draft task in progress");
      expect(
        screen.getByText("Open a repository to enable planning")
      ).toBeInTheDocument();

      // 4. Project reopens
      act(() => {
        bridge.emitMessage({
          messageType: "snapshot",
          snapshot: {
            streamId: "stream-s1",
            lastSequence: 3,
            project: {
              id: "proj-1",
              displayName: "repo-1",
              canonicalRoot: "D:\\code\\repo-1",
              trusted: true
            },
            activeOperations: []
          }
        });
      });

      expect(textarea).toBeEnabled();
      expect(textarea.value).toBe("Draft task in progress");
    });
  });

  describe("4. Opened Repository Metadata & Trust Display Verification", () => {
    it("renders trusted repository with canonical root tooltip and Trusted badge", async () => {
      const bridge = createMockBridge();
      render(<App bridge={bridge} />);
      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

      act(() => {
        bridge.emitMessage({
          messageType: "snapshot",
          snapshot: {
            streamId: "s-trusted",
            lastSequence: 1,
            project: {
              id: "proj-trusted",
              displayName: "kesharon-core",
              canonicalRoot: "D:\\Proiecte AI\\Kesharon Agent",
              trusted: true
            },
            activeOperations: []
          }
        });
      });

      const repoTitle = screen.getByText("kesharon-core");
      const canonicalRoot = screen.getByTitle("D:\\Proiecte AI\\Kesharon Agent");
      const trustBadge = screen.getByText("Trusted");

      expect(repoTitle).toBeInTheDocument();
      expect(canonicalRoot).toBeInTheDocument();
      expect(canonicalRoot).toHaveTextContent("D:\\Proiecte AI\\Kesharon Agent");
      expect(trustBadge).toHaveClass("trust-badge--trusted");
    });

    it("renders untrusted repository with Untrusted (Sandbox) badge", async () => {
      const bridge = createMockBridge();
      render(<App bridge={bridge} />);
      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

      act(() => {
        bridge.emitMessage({
          messageType: "snapshot",
          snapshot: {
            streamId: "s-untrusted",
            lastSequence: 1,
            project: {
              id: "proj-untrusted",
              displayName: "third-party-lib",
              canonicalRoot: "C:\\Users\\Public\\third-party-lib",
              trusted: false
            },
            activeOperations: []
          }
        });
      });

      const repoTitle = screen.getByText("third-party-lib");
      const canonicalRoot = screen.getByTitle("C:\\Users\\Public\\third-party-lib");
      const trustBadge = screen.getByText("Untrusted (Sandbox)");

      expect(repoTitle).toBeInTheDocument();
      expect(canonicalRoot).toBeInTheDocument();
      expect(trustBadge).toHaveClass("trust-badge--untrusted");
    });

    it("allows switching repository via 'Open another repository' button", async () => {
      const bridge = createMockBridge({
        chooseProjectDirectory: vi.fn(async () => "D:\\code\\another-repo"),
        openProject: vi.fn(async (path: string) => ({
          requestId: "req-open-another",
          project: {
            id: "proj-another",
            displayName: "another-repo",
            canonicalRoot: path,
            trusted: true
          }
        }))
      });

      render(<App bridge={bridge} />);
      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

      // Set initial project
      act(() => {
        bridge.emitMessage({
          messageType: "snapshot",
          snapshot: {
            streamId: "s-1",
            lastSequence: 1,
            project: {
              id: "proj-1",
              displayName: "first-repo",
              canonicalRoot: "D:\\code\\first-repo",
              trusted: true
            },
            activeOperations: []
          }
        });
      });

      expect(screen.getByText("first-repo")).toBeInTheDocument();

      // Click "Open another repository"
      const switchBtn = screen.getByRole("button", { name: "Open another repository" });
      await act(async () => {
        fireEvent.click(switchBtn);
      });

      expect(bridge.chooseProjectDirectory).toHaveBeenCalled();
      expect(bridge.openProject).toHaveBeenCalledWith("D:\\code\\another-repo");
      expect(await screen.findByText("another-repo")).toBeInTheDocument();
      expect(screen.getByText("D:\\code\\another-repo")).toBeInTheDocument();
    });
  });

  describe("5. Reconnection & Stream Resilience During User Interaction", () => {
    it("preserves task input and smoothly transitions UI across reconnect bursts", async () => {
      const bridge = createMockBridge();
      render(<App bridge={bridge} />);
      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();

      // 1. Initial snapshot with active project
      act(() => {
        bridge.emitMessage({
          messageType: "snapshot",
          snapshot: {
            streamId: "stream-live-1",
            lastSequence: 5,
            project: {
              id: "proj-live",
              displayName: "live-app",
              canonicalRoot: "D:\\code\\live-app",
              trusted: true
            },
            activeOperations: []
          }
        });
      });

      const textarea = screen.getByPlaceholderText(
        "What should change in this repository?"
      ) as HTMLTextAreaElement;
      fireEvent.change(textarea, { target: { value: "Drafting feature during unstable connection" } });
      expect(textarea.value).toBe("Drafting feature during unstable connection");

      // 2. Stream reconnect event
      act(() => {
        bridge.emitUpdate({ type: "reconnecting" });
      });

      expect(await screen.findByText("Reconnecting to daemon")).toBeInTheDocument();
      // Textarea retains entered text
      expect(textarea.value).toBe("Drafting feature during unstable connection");

      // 3. Authoritative snapshot arrives upon reconnect
      act(() => {
        bridge.emitMessage({
          messageType: "snapshot",
          snapshot: {
            streamId: "stream-live-2",
            lastSequence: 1,
            project: {
              id: "proj-live",
              displayName: "live-app",
              canonicalRoot: "D:\\code\\live-app",
              trusted: true
            },
            activeOperations: []
          }
        });
      });

      expect(await screen.findByText("Daemon ready")).toBeInTheDocument();
      expect(textarea.value).toBe("Drafting feature during unstable connection");
      expect(screen.getByText("live-app")).toBeInTheDocument();
    });
  });
});
