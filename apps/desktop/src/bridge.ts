import { invoke } from "@tauri-apps/api/core";
import {
  type HealthSnapshot,
  decodeHealthResponse
} from "@kesharon/protocol";

import type { DesktopBridge } from "./App";

export const tauriBridge: DesktopBridge = {
  async getHealth(): Promise<HealthSnapshot> {
    const response = await invoke<unknown>("daemon_health");
    return decodeHealthResponse(response);
  }
};
