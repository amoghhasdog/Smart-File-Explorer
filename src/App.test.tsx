import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string) => (cmd === "app_version" ? "0.1.0" : null)),
}));

import App from "./App";

describe("App shell", () => {
  it("renders the title and the backend version from IPC", async () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: /smart file explorer/i })).toBeTruthy();
    expect(await screen.findByText(/backend v0\.1\.0/)).toBeTruthy();
  });
});
