import { invoke } from "@tauri-apps/api/core";
import type { AgentSlug } from "./api";

// Mirrors `src/gui_prompt_overrides.rs`. Kept beside the manager instead of in
// `api.ts` so the two increments do not edit one shared file.

export type OverrideContext =
  | { kind: "global" }
  | { kind: "project"; project_id: string }
  | { kind: "feature"; project_id: string; feature_id: string };

export type OverrideLayer = "feature" | "project" | "global" | "built_in";
export type OverrideScope = Exclude<OverrideLayer, "built_in">;

export interface ScopeOption {
  scope: OverrideScope;
  label: string;
  available: boolean;
  reason: string | null;
}

export interface StoredOverride {
  scope: OverrideScope;
  /** null is the shared template for every harness. */
  harness: AgentSlug | null;
  template: string;
}

export interface OverrideRow {
  id: string;
  title: string;
  summary: string;
  placeholders: string[];
  source: OverrideLayer;
  source_harness: AgentSlug | null;
  effective_template: string;
  default_template: string;
  stored: StoredOverride[];
  /** Fingerprint of this prompt's stored overrides in the context. */
  revision: string;
}

export interface OverridesView {
  context: OverrideContext;
  context_label: string;
  repo: string | null;
  workdir: string | null;
  harness: AgentSlug;
  scopes: ScopeOption[];
  project_config_error: string | null;
  rows: OverrideRow[];
}

export interface SaveOverride {
  context: OverrideContext;
  prompt_id: string;
  scope: OverrideScope;
  harness: AgentSlug | null;
  template: string;
  revision: string;
  view_harness: AgentSlug | null;
}

export type ClearOverride = Omit<SaveOverride, "template">;


export const HARNESS_LABELS: Record<AgentSlug, string> = {
  claude: "Claude", codex: "Codex", pi: "Pi",
  gemini: "Gemini", cursor: "Cursor", // FORK
};

export const SCOPE_LABELS: Record<OverrideLayer, string> = {
  feature: "Feature", project: "Project (aiw.json)", global: "Global", built_in: "Built-in default",
};

export function slotLabel(scope: OverrideLayer, harness: AgentSlug | null): string {
  if (scope === "built_in") return SCOPE_LABELS.built_in;
  return `${SCOPE_LABELS[scope]} · ${harness ? HARNESS_LABELS[harness] : "all harnesses"}`;
}
