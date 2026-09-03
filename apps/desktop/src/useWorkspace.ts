import { useCallback, useEffect, useReducer } from "react";
import type {
  DaemonEvent,
  DaemonStreamUpdate,
  HealthSnapshot,
  ProjectSnapshot,
  StreamMessage,
  WorkspaceSnapshot
} from "@kesharon/protocol";
import type { DesktopBridge } from "./bridge";

export type ConnectionState =
  | { kind: "checking" }
  | { kind: "ready"; health?: HealthSnapshot }
  | { kind: "reconnecting" }
  | { kind: "unavailable"; message?: string };

export interface OpeningOperationState {
  requestId: string;
  path: string;
  status: "opening" | "cancelling";
}

export interface WorkspaceState {
  connection: ConnectionState;
  project: ProjectSnapshot | null;
  openingOperation: OpeningOperationState | null;
  openingError: string | null;
  taskGoal: string;
  streamId: string | null;
  lastSequence: number;
}

type WorkspaceAction =
  | { type: "SET_CONNECTION"; connection: ConnectionState }
  | { type: "APPLY_SNAPSHOT"; snapshot: WorkspaceSnapshot }
  | { type: "APPLY_EVENT"; event: DaemonEvent }
  | { type: "START_OPENING"; requestId: string; path: string }
  | { type: "PROJECT_OPENED"; project: ProjectSnapshot }
  | { type: "OPENING_FAILED"; message: string }
  | { type: "SET_OPENING_STATUS"; status: "opening" | "cancelling" }
  | { type: "SET_TASK_GOAL"; goal: string }
  | { type: "CLEAR_ERROR" };

const initialState: WorkspaceState = {
  connection: { kind: "checking" },
  project: null,
  openingOperation: null,
  openingError: null,
  taskGoal: "",
  streamId: null,
  lastSequence: 0
};

function workspaceReducer(
  state: WorkspaceState,
  action: WorkspaceAction
): WorkspaceState {
  switch (action.type) {
    case "SET_CONNECTION": {
      return {
        ...state,
        connection: action.connection
      };
    }

    case "APPLY_SNAPSHOT": {
      const activeOpen = action.snapshot.activeOperations.find(
        (op) => op.kind === "openProject"
      );

      let openingOperation: OpeningOperationState | null = null;
      if (activeOpen) {
        openingOperation = {
          requestId: activeOpen.requestId,
          path: state.openingOperation?.path || "",
          status: state.openingOperation?.status || "opening"
        };
      }

      return {
        ...state,
        connection:
          state.connection.kind === "reconnecting" ||
          state.connection.kind === "checking"
            ? { kind: "ready" }
            : state.connection,
        project: action.snapshot.project,
        openingOperation,
        openingError: null,
        streamId: action.snapshot.streamId,
        lastSequence: action.snapshot.lastSequence
      };
    }

    case "APPLY_EVENT": {
      const { payload, sequence, streamId } = action.event;
      let nextProject = state.project;
      let nextOpening = state.openingOperation;
      let nextError = state.openingError;

      switch (payload.type) {
        case "operationStarted": {
          if (payload.kind === "openProject") {
            nextOpening = {
              requestId: payload.requestId,
              path: state.openingOperation?.path || "",
              status: "opening"
            };
          }
          break;
        }

        case "projectOpened": {
          nextProject = payload.project;
          nextOpening = null;
          nextError = null;
          break;
        }

        case "operationCancelled": {
          nextOpening = null;
          break;
        }

        case "operationFailed": {
          nextOpening = null;
          nextError = payload.message;
          break;
        }
      }

      return {
        ...state,
        project: nextProject,
        openingOperation: nextOpening,
        openingError: nextError,
        streamId,
        lastSequence: sequence
      };
    }

    case "START_OPENING": {
      return {
        ...state,
        openingOperation: {
          requestId: action.requestId,
          path: action.path,
          status: "opening"
        },
        openingError: null
      };
    }

    case "PROJECT_OPENED": {
      return {
        ...state,
        project: action.project,
        openingOperation: null,
        openingError: null
      };
    }

    case "OPENING_FAILED": {
      return {
        ...state,
        openingOperation: null,
        openingError: action.message
      };
    }

    case "SET_OPENING_STATUS": {
      if (!state.openingOperation) {
        return state;
      }
      return {
        ...state,
        openingOperation: {
          ...state.openingOperation,
          status: action.status
        }
      };
    }

    case "SET_TASK_GOAL": {
      return {
        ...state,
        taskGoal: action.goal
      };
    }

    case "CLEAR_ERROR": {
      return {
        ...state,
        openingError: null
      };
    }

    default:
      return state;
  }
}

export function useWorkspace(bridge: DesktopBridge) {
  const [state, dispatch] = useReducer(workspaceReducer, initialState);

  const handleStreamUpdate = useCallback(
    (update: DaemonStreamUpdate | StreamMessage) => {
      if ("messageType" in update) {
        // Direct StreamMessage support
        dispatch({ type: "SET_CONNECTION", connection: { kind: "ready" } });
        if (update.messageType === "snapshot") {
          dispatch({ type: "APPLY_SNAPSHOT", snapshot: update.snapshot });
        } else if (update.messageType === "event") {
          dispatch({ type: "APPLY_EVENT", event: update.event });
        }
        return;
      }

      if (update.type === "reconnecting") {
        dispatch({
          type: "SET_CONNECTION",
          connection: { kind: "reconnecting" }
        });
        return;
      }

      if (update.type === "unavailable") {
        dispatch({
          type: "SET_CONNECTION",
          connection: { kind: "unavailable", message: update.message }
        });
        return;
      }

      if (update.type === "message") {
        dispatch({ type: "SET_CONNECTION", connection: { kind: "ready" } });
        if (update.message.messageType === "snapshot") {
          dispatch({
            type: "APPLY_SNAPSHOT",
            snapshot: update.message.snapshot
          });
        } else if (update.message.messageType === "event") {
          dispatch({
            type: "APPLY_EVENT",
            event: update.message.event
          });
        }
      }
    },
    []
  );

  useEffect(() => {
    let active = true;
    let unsubscribeStream: (() => void) | null = null;

    bridge
      .getHealth()
      .then((health) => {
        if (active) {
          dispatch({
            type: "SET_CONNECTION",
            connection: { kind: "ready", health }
          });
        }
      })
      .catch((err: unknown) => {
        if (active) {
          const message =
            err instanceof Error ? err.message : "Failed to connect to daemon";
          dispatch({
            type: "SET_CONNECTION",
            connection: { kind: "unavailable", message }
          });
        }
      });

    bridge
      .subscribeDaemonEvents((update) => {
        if (active) {
          handleStreamUpdate(update);
        }
      })
      .then((unsub) => {
        if (active) {
          unsubscribeStream = unsub;
        } else {
          unsub();
        }
      })
      .catch((err: unknown) => {
        console.error("Failed to subscribe to daemon events", err);
      });

    return () => {
      active = false;
      unsubscribeStream?.();
    };
  }, [bridge, handleStreamUpdate]);

  const chooseAndOpen = useCallback(async () => {
    dispatch({ type: "CLEAR_ERROR" });
    try {
      const path = await bridge.chooseProjectDirectory();
      if (!path) {
        return;
      }
      const requestId = `req-client-open-${Date.now()}`;
      dispatch({ type: "START_OPENING", requestId, path });
      const result = await bridge.openProject(path);
      dispatch({ type: "PROJECT_OPENED", project: result.project });
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : "Failed to open project";
      dispatch({ type: "OPENING_FAILED", message });
    }
  }, [bridge]);

  const cancelOpening = useCallback(async () => {
    if (!state.openingOperation) {
      return;
    }
    const { requestId } = state.openingOperation;
    dispatch({ type: "SET_OPENING_STATUS", status: "cancelling" });

    try {
      await bridge.cancelRequest(requestId);
    } catch (err: unknown) {
      console.error("Failed to cancel opening request", err);
    }
  }, [bridge, state.openingOperation]);

  const setTaskGoal = useCallback((goal: string) => {
    dispatch({ type: "SET_TASK_GOAL", goal });
  }, []);

  const clearError = useCallback(() => {
    dispatch({ type: "CLEAR_ERROR" });
  }, []);

  return {
    state,
    chooseAndOpen,
    cancelOpening,
    setTaskGoal,
    clearError
  };
}
