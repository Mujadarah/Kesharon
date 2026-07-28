import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { App, type DesktopBridge } from "./App";

const readyBridge: DesktopBridge = {
  async getHealth() {
    return {
      requestId: "request-ui-1",
      status: "ready",
      protocolVersion: 1
    };
  }
};

describe("App", () => {
  it("shows the code-first workspace and verified daemon state", async () => {
    render(<App bridge={readyBridge} />);

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
    const unavailableBridge: DesktopBridge = {
      async getHealth() {
        throw new Error("Named pipe is unavailable");
      }
    };

    render(<App bridge={unavailableBridge} />);

    expect(await screen.findByText("Daemon unavailable")).toBeInTheDocument();
    expect(
      screen.getByText("Restart Kesharon, then check the connection again.")
    ).toBeInTheDocument();
  });
});
