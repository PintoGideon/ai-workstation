import { invoke } from "@tauri-apps/api/core";
import type { SessionTarget } from "./api";

// Mirrors `src/gui_contract/session_sidebar.rs`: the agent sidebar the TUI
// draws beside an embedded agent pane, built by the same shared assembly.

export type SidebarSectionKind =
  | "status" | "usage" | "context" | "issue" | "pr"
  | "work" | "summary" | "prompt" | "todos" | "active_todo";

export type SidebarTone =
  | "plain" | "muted" | "state_active" | "state_idle" | "state_stopped"
  | "waiting" | "busy" | "ready" | "generating" | "hint" | "todo" | "detail";

export type UsageLevel = "low" | "medium" | "high";
export type TodoItemState = "done" | "active" | "pending";

export type SidebarLine =
  | { kind: "field"; label: string; value: string; tone: SidebarTone; emphasised: boolean }
  | { kind: "text"; text: string; tone: SidebarTone; emphasised: boolean }
  | { kind: "bar"; label: string; used_percent: number; level: UsageLevel; reset: string }
  | { kind: "progress"; done: number; total: number }
  | { kind: "item"; state: TodoItemState; text: string }
  | { kind: "more"; text: string };

export interface SidebarContextMeter {
  percent: number;
  used_tokens: number;
  limit_tokens: number;
  band: "normal" | "warning" | "critical";
  estimated: boolean;
  stale: boolean;
  fresh_context_hint: boolean;
}

export type SidebarAction =
  | { kind: "reuse_prompt"; prompt: string }
  | { kind: "complete_todo"; todo_id: string }
  | { kind: "supervised_edits" };

export interface SessionSidebarSection {
  kind: SidebarSectionKind;
  title: string;
  lines: SidebarLine[];
  context?: SidebarContextMeter;
  actions: SidebarAction[];
}

export interface SessionSidebarView {
  session_id: string;
  harness: "claude" | "codex" | "pi" | "gemini" | "cursor"; // FORK
  title: string;
  sections: SessionSidebarSection[];
  notes: string[];
}

export function sessionSidebar(target: SessionTarget): Promise<SessionSidebarView> {
  return invoke("session_sidebar", { target });
}

export function sessionSidebarCompleteTodo(target: SessionTarget, todoId: string): Promise<string> {
  return invoke("session_sidebar_complete_todo", { target, request: { todo_id: todoId } });
}
