import { Channel, invoke } from "@tauri-apps/api/core";
import {
  type CancellationResult,
  type DaemonStreamUpdate,
  type HealthSnapshot,
  type ProjectOpenResult,
  decodeCancellationResponse,
  decodeDaemonStreamUpdate,
  decodeHealthResponse,
  decodeProjectOpenResponse
} from "@kesharon/protocol";

export interface DesktopBridge {
  getHealth(): Promise<HealthSnapshot>;
  chooseProjectDirectory(): Promise<string | null>;
  openProject(path: string): Promise<ProjectOpenResult>;
  cancelRequest(targetRequestId: string): Promise<CancellationResult>;
  subscribeDaemonEvents(
    onUpdate: (update: DaemonStreamUpdate) => void
  ): Promise<() => void>;
}

export const tauriBridge: DesktopBridge = {
  async getHealth(): Promise<HealthSnapshot> {
    const response = await invoke<unknown>("daemon_health");
    return decodeHealthResponse(response);
  },

  async chooseProjectDirectory(): Promise<string | null> {
    const result = await invoke<string | null>("choose_project_directory");
    return typeof result === "string" ? result : null;
  },

  async openProject(path: string): Promise<ProjectOpenResult> {
    const response = await invoke<unknown>("daemon_open_project", { path });
    return decodeProjectOpenResponse(response);
  },

  async cancelRequest(targetRequestId: string): Promise<CancellationResult> {
    const response = await invoke<unknown>("daemon_cancel_request", {
      targetRequestId
    });
    return decodeCancellationResponse(response);
  },

  async subscribeDaemonEvents(
    onUpdate: (update: DaemonStreamUpdate) => void
  ): Promise<() => void> {
    const channel = new Channel<unknown>((raw) => {
      try {
        const decoded = decodeDaemonStreamUpdate(raw);
        onUpdate(decoded);
      } catch (error) {
        console.error("Failed to decode daemon stream update", error);
      }
    });

    await invoke("subscribe_daemon_events", { onEvent: channel });

    return () => {
      // Channel lifecycle cleanup
    };
  }
};
