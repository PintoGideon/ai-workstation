// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import PrMarkdown, { PrDescription } from "../src/PrMarkdown";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const image = { data_url: "data:image/png;base64,AA==", width: 4, height: 3 };
const props = { workflowId: "triage", identity: "triage:12:head", onOpenImage: vi.fn() };

// FORK(no-cloud): AIW downloads no images itself. Images become placeholders
// whose link only opens in the user's browser on an explicit click.
it("renders Markdown/reference/HTML images as placeholders, fetches nothing and sanitizes active HTML", async () => {
  vi.mocked(invoke).mockResolvedValue(undefined);
  const source = `Before ![Inline](./ready.png) after\n\n![Reference][picture]\n\n[picture]: https://github.com/user-attachments/assets/reference\n\n<img alt="HTML upload" src="https://github.com/user-attachments/assets/html" onerror="alert(1)" />\n\n\`![Code](https://example.com/code.png)\`\n\n\`\`\`\n<img src="https://example.com/fenced.png">\n\`\`\`\n\n<!-- <img src="https://example.com/comment.png"> -->\n<script><img src="https://example.com/script.png"></script>\n<iframe src="https://example.com/frame"></iframe>`;
  const { container } = render(<PrMarkdown {...props} source={source} />);
  expect(await screen.findByText("[image: HTML upload]", { exact: false })).toBeTruthy();
  expect(screen.getByText("[image: Reference]", { exact: false })).toBeTruthy();
  expect(screen.getByText("[image: Inline]", { exact: false })).toBeTruthy();
  expect(container.querySelectorAll("img")).toHaveLength(0);
  expect(container.querySelector("script,iframe,[onerror]")).toBeNull();
  expect(vi.mocked(invoke)).not.toHaveBeenCalled();
});

it("opens an image link in the browser only when asked", async () => {
  vi.mocked(invoke).mockResolvedValue(undefined);
  render(<PrMarkdown {...props} source="![Upload](https://github.com/user-attachments/assets/file) ![Local](./local.png)" />);
  const buttons = await screen.findAllByRole("button", { name: "Open in browser" });
  expect(buttons).toHaveLength(1);
  expect(vi.mocked(invoke)).not.toHaveBeenCalled();
  fireEvent.click(buttons[0]);
  expect(vi.mocked(invoke)).toHaveBeenCalledWith("screenshots_open_browser", {
    url: "https://github.com/user-attachments/assets/file",
  });
});

it("shows a browser-open failure", async () => {
  vi.mocked(invoke).mockRejectedValue({ message: "Unsupported or unsafe link" });
  render(<PrMarkdown {...props} source="![Upload](https://github.com/user-attachments/assets/file)" />);
  fireEvent.click(await screen.findByRole("button", { name: "Open in browser" }));
  expect(await screen.findByRole("alert")).toBeTruthy();
});

it("does not apply an old description after switching PRs", async () => {
  let complete!: (value: string) => void;
  const old = new Promise<string>((resolve) => { complete = resolve; });
  vi.mocked(invoke).mockImplementation((command, args) => command === "screenshots_pr_document"
    ? (args as { workflowId: string }).workflowId === "old" ? old : Promise.resolve("New description")
    : Promise.resolve(image));
  const view = render(<PrDescription {...props} workflowId="old" identity="old" />);
  view.rerender(<PrDescription {...props} />);
  expect(await screen.findByText("New description")).toBeTruthy();
  complete("Old description");
  await waitFor(() => expect(screen.queryByText("Old description")).toBeNull());
});
