// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import SessionSidebar from "../src/SessionSidebar";
import { resetSidebarPrefsForTest } from "../src/sidebarPrefs";
import type { SessionSidebarView } from "../src/sessionSidebarApi";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const target = { project_id: "project", feature_id: "feature", session_id: "claude" };
const FULL_PROMPT = "Add the session sidebar to the GUI agent tabs and keep the composer draft";

const field = (label: string, value: string, tone = "plain", emphasised = false) =>
  ({ kind: "field" as const, label, value, tone: tone as never, emphasised });

function fullView(): SessionSidebarView {
  return {
    session_id: "claude",
    harness: "claude",
    title: "Claude Sidebar",
    notes: ["Attention reasons, running tool calls and Codex live events are reported only to a running TUI, so this panel does not show them."],
    sections: [
      { kind: "status", title: "Status", actions: [], lines: [
        field("Activity", "Ready", "ready", true), field("Input", "12.3k tokens"), field("Cost", "$0.42"), field("Model", "claude-opus"),
      ] },
      { kind: "usage", title: "Usage", actions: [], lines: [
        { kind: "bar", label: "5h", used_percent: 62, level: "medium", reset: "3h" },
        { kind: "bar", label: "7d", used_percent: 91, level: "high", reset: "2d" },
      ] },
      { kind: "context", title: "Context", actions: [], lines: [
        { kind: "text", text: "Ctx ~85% CRITICAL STALE · 170,000", tone: "plain", emphasised: false },
      ], context: { percent: 85, used_tokens: 170000, limit_tokens: 200000, band: "critical", estimated: true, stale: true, fresh_context_hint: true } },
      { kind: "issue", title: "Issue", actions: [], lines: [field("Repository", "github.com/acme/widget"), field("Issue", "#42")] },
      { kind: "pr", title: "Pull request", actions: [], lines: [field("PR", "#77 · 2 open")] },
      { kind: "work", title: "Work", actions: [{ kind: "supervised_edits" }], lines: [field("State", "waiting for diff review", "waiting", true)] },
      { kind: "summary", title: "Summary", actions: [], lines: [{ kind: "text", text: "Builds the GUI agent sidebar", tone: "plain", emphasised: false }] },
      { kind: "prompt", title: "Prompt", actions: [{ kind: "reuse_prompt", prompt: FULL_PROMPT }], lines: [
        { kind: "text", text: "Add the session sidebar to the GUI agent tabs a…", tone: "plain", emphasised: false },
      ] },
      { kind: "todos", title: "Todos", actions: [], lines: [
        { kind: "progress", done: 2, total: 5 },
        { kind: "item", state: "done", text: "Read the spec" },
        { kind: "item", state: "active", text: "Write the panel" },
        { kind: "item", state: "pending", text: "Screenshots" },
        { kind: "more", text: "+2 more" },
      ] },
      { kind: "active_todo", title: "Active TODO", actions: [{ kind: "complete_todo", todo_id: "todo-1" }], lines: [
        { kind: "text", text: "Ship the panel", tone: "plain", emphasised: false }, field("State", "open", "plain", true),
      ] },
    ],
  };
}

function sparseView(): SessionSidebarView {
  return {
    session_id: "claude", harness: "pi", title: "Pi Sidebar", notes: [],
    sections: [
      { kind: "status", title: "Status", actions: [], lines: [field("Activity", "Ready", "ready", true)] },
    ],
  };
}

let current: SessionSidebarView;
let client: QueryClient;

beforeEach(() => {
  window.localStorage.clear();
  resetSidebarPrefsForTest();
  current = fullView();
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "session_sidebar") return current;
    if (command === "session_sidebar_complete_todo") return "Marked referenced TODO complete";
    throw new Error(`Unexpected command: ${command}`);
  });
});

afterEach(() => {
  client?.clear();
  cleanup();
  vi.clearAllMocks();
});

function mount(handlers: Partial<Record<"onReusePrompt" | "onSupervisedEdits" | "onFreshSession", ReturnType<typeof vi.fn>>> = {}) {
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const props = {
    onReusePrompt: handlers.onReusePrompt ?? vi.fn(),
    onSupervisedEdits: handlers.onSupervisedEdits ?? vi.fn(),
    onFreshSession: handlers.onFreshSession ?? vi.fn(),
  };
  render(<QueryClientProvider client={client}><SessionSidebar target={target} {...props} /></QueryClientProvider>);
  return props;
}

const sidebar = () => screen.getByRole("complementary");
const section = (title: string) => within(sidebar()).getByRole("region", { name: title });
const sectionTitles = () => within(sidebar()).queryAllByRole("region").map((node) => node.getAttribute("aria-label"));
const sidebarCalls = () => vi.mocked(invoke).mock.calls.filter(([command]) => command === "session_sidebar");

describe("session sidebar", () => {
  it("opens fresh context from the meter and hands the new session draft to navigation", async () => {
    const previous = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === "fresh_context_preview") return { revision: "fresh", prompt: "Continue", label: "Fresh Context" };
      if (command === "fresh_context_start") return { target: { ...target, session_id: "fresh-session" }, draft_prompt: "Continue" };
      return previous(command, args);
    });
    const { onFreshSession } = mount();
    fireEvent.click(await screen.findByRole("button", { name: "Fresh context" }));
    await screen.findByRole("textbox", { name: "Continuation prompt" });
    fireEvent.click(screen.getByRole("button", { name: "Start fresh context" }));
    await waitFor(() => expect(onFreshSession).toHaveBeenCalledWith({ ...target, session_id: "fresh-session" }, "Continue"));
    expect(screen.queryByRole("dialog", { name: "Fresh context" })).toBeNull();
  });

  it("shows every section with content in the TUI's order under a per-harness title", async () => {
    mount();
    expect(await screen.findByRole("heading", { name: "Claude Sidebar" })).toBeTruthy();
    expect(sidebar().getAttribute("data-harness")).toBe("claude");
    expect(sectionTitles()).toEqual([
      "Status", "Usage", "Context", "Issue", "Pull request", "Work", "Summary", "Prompt", "Todos", "Active TODO",
    ]);
    expect(sidebarCalls()[0]).toEqual(["session_sidebar", { target }]);
    const status = section("Status");
    expect(within(status).getByText("Ready").className).toContain("tone-ready");
    expect(within(status).getByText("Ready").className).toContain("sb-strong");
    expect(within(status).getByText("claude-opus")).toBeTruthy();
    expect(within(section("Pull request")).getByText("#77 · 2 open")).toBeTruthy();
    expect(screen.getByText(/reported only to a running TUI/)).toBeTruthy();
  });

  it("hides empty sections", async () => {
    current = sparseView();
    mount();
    expect(await screen.findByRole("heading", { name: "Pi Sidebar" })).toBeTruthy();
    expect(sectionTitles()).toEqual(["Status"]);
    expect(screen.queryByText(/running TUI/)).toBeNull();
  });

  it("draws usage bars in their utilization bands with time to reset", async () => {
    mount();
    const usage = await waitFor(() => section("Usage"));
    const fiveHour = within(usage).getByRole("meter", { name: "5h usage" });
    expect(fiveHour.getAttribute("aria-valuenow")).toBe("62");
    expect(fiveHour.querySelector(".sb-fill")?.className).toContain("usage-medium");
    expect(within(usage).getByRole("meter", { name: "7d usage" }).querySelector(".sb-fill")?.className).toContain("usage-high");
    expect(within(usage).getByText("3h")).toBeTruthy();
    expect(within(usage).getByText("91%")).toBeTruthy();
  });

  it("colours the context section by band and labels estimated and stale readings", async () => {
    mount();
    const context = await waitFor(() => section("Context"));
    expect(context.getAttribute("data-band")).toBe("critical");
    expect(within(context).getByRole("meter", { name: "Context window used" }).getAttribute("aria-valuenow")).toBe("85");
    expect(within(context).getByText("Ctx ~85% CRITICAL STALE · 170,000")).toBeTruthy();
    expect(within(context).getByText("estimated")).toBeTruthy();
    expect(within(context).getByText("stale")).toBeTruthy();
    expect(within(context).getByRole("button", { name: "Fresh context" })).toBeTruthy();
    expect(within(context).queryByText(/The TUI offers a fresh context/)).toBeNull();
  });

  it("renders the agent todo list with its progress bar and states", async () => {
    mount();
    const todos = await waitFor(() => section("Todos"));
    expect(within(todos).getByRole("progressbar", { name: "Todos done" }).getAttribute("aria-valuenow")).toBe("2");
    expect(within(todos).getByText("2/5")).toBeTruthy();
    expect(within(todos).getByText("Write the panel").parentElement?.className).toContain("sb-item-active");
    expect(within(todos).getByText("Read the spec").parentElement?.className).toContain("sb-item-done");
    expect(within(todos).getByText("+2 more")).toBeTruthy();
  });

  it("clamps the prompt, shows it in full on request and reuses it in the draft", async () => {
    const { onReusePrompt } = mount();
    const prompt = await waitFor(() => section("Prompt"));
    expect(within(prompt).getByText(/agent tabs a…$/)).toBeTruthy();
    expect(within(prompt).queryByLabelText("Full prompt")).toBeNull();
    fireEvent.click(within(prompt).getByRole("button", { name: "View" }));
    expect(within(prompt).getByLabelText("Full prompt").textContent).toBe(FULL_PROMPT);
    const reuse = within(prompt).getByRole("button", { name: "Reuse" });
    expect(reuse.getAttribute("title")).toContain("TUI: leader l");
    fireEvent.click(reuse);
    expect(onReusePrompt).toHaveBeenCalledWith(FULL_PROMPT);
  });

  it("opens supervised edits through the GUI's own panel", async () => {
    const { onSupervisedEdits } = mount();
    fireEvent.click(await waitFor(() => within(section("Work")).getByRole("button", { name: "Review" })));
    expect(onSupervisedEdits).toHaveBeenCalledTimes(1);
  });

  it("hides cached actions when a refresh reports a deleted session", async () => {
    mount();
    await waitFor(() => section("Active TODO"));
    vi.mocked(invoke).mockRejectedValue({ kind: "not_found", message: "Session deleted elsewhere" });
    await act(async () => { await client.invalidateQueries({ queryKey: ["session-sidebar"] }); });
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Session deleted elsewhere");
    expect(screen.queryByRole("region", { name: "Active TODO" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Reuse" })).toBeNull();
  });

  it("completes the active TODO only after confirmation", async () => {
    mount();
    const active = await waitFor(() => section("Active TODO"));
    const complete = within(active).getByRole("button", { name: "Complete" });
    expect(complete.getAttribute("title")).toContain("TUI: leader z");
    fireEvent.click(complete);
    const confirm = within(active).getByRole("group", { name: "Confirm TODO completion" });
    fireEvent.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "session_sidebar_complete_todo")).toBe(false);

    fireEvent.click(within(active).getByRole("button", { name: "Complete" }));
    fireEvent.click(within(within(active).getByRole("group")).getByRole("button", { name: "Complete" }));
    expect(await within(active).findByText("Marked referenced TODO complete")).toBeTruthy();
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("session_sidebar_complete_todo", { target, request: { todo_id: "todo-1" } });
  });

  it("reports a refused completion without hiding the TODO", async () => {
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "session_sidebar") return current;
      throw { kind: "conflict", message: "This session is no longer linked to that TODO; refresh and retry" };
    });
    mount();
    const active = await waitFor(() => section("Active TODO"));
    fireEvent.click(within(active).getByRole("button", { name: "Complete" }));
    fireEvent.click(within(within(active).getByRole("group")).getByRole("button", { name: "Complete" }));
    expect((await within(active).findByRole("alert")).textContent).toContain("no longer linked");
    expect(within(active).getByText("Ship the panel")).toBeTruthy();
  });

  it("collapses to a rail, stops polling and remembers the choice", async () => {
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Hide agent sidebar" }));
    const rail = screen.getByRole("button", { name: "Show agent sidebar" });
    expect(rail.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("region")).toBeNull();
    expect(window.localStorage.getItem("aiw.gui.collapsed.sessionSidebar")).toBe("1");
    const calls = sidebarCalls().length;
    await act(async () => { await client.refetchQueries(); });
    expect(sidebarCalls().length).toBe(calls);

    // A new window (fresh module state) starts collapsed from storage.
    cleanup();
    resetSidebarPrefsForTest();
    mount();
    expect(screen.getByRole("button", { name: "Show agent sidebar" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Show agent sidebar" }));
    expect(await screen.findByRole("heading", { name: "Claude Sidebar" })).toBeTruthy();
    expect(window.localStorage.getItem("aiw.gui.collapsed.sessionSidebar")).toBe("0");
  });

  it("still works when storage refuses writes", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("denied"); });
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Hide agent sidebar" }));
    expect(screen.getByRole("button", { name: "Show agent sidebar" })).toBeTruthy();
    setItem.mockRestore();
  });

  it("says why when the session is gone", async () => {
    vi.mocked(invoke).mockImplementation(async () => {
      throw { kind: "not_found", message: "The selected session no longer exists" };
    });
    mount();
    expect((await screen.findByRole("alert")).textContent).toBe("The selected session no longer exists");
  });
});
