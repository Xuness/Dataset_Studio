import { Database, List, Gauge, Undo2, Plug, FileText } from "lucide-react";
import { SystemPromptsPage } from "./system-prompts/SystemPromptsPage.js";
import { LlmSettingsPage } from "./llm/LlmSettingsPage.js";
import { EditingSettingsPage } from "./EditingSettingsPage.js";
import { CacheSettingsPage } from "./CacheSettingsPage.js";
import { CacheManagerPage } from "./CacheManagerPage.js";
import { PerformanceSettingsPage } from "./PerformanceSettingsPage.js";

/** Register a settings page here; it stays outside the project's business views. */
export const settingsPages = [
  { id: "llm", title: "API 与模型", Icon: Plug, Component: LlmSettingsPage },
  {
    id: "system-prompts",
    title: "System Prompt",
    Icon: FileText,
    Component: SystemPromptsPage,
  },
  {
    id: "editing",
    title: "编辑与撤销",
    Icon: Undo2,
    Component: EditingSettingsPage,
  },
  {
    id: "cache",
    title: "缓存与存储",
    Icon: Database,
    Component: CacheSettingsPage,
  },
  {
    id: "cache-manager",
    title: "缓存管理",
    Icon: List,
    Component: CacheManagerPage,
  },
  {
    id: "performance",
    title: "性能与任务",
    Icon: Gauge,
    Component: PerformanceSettingsPage,
  },
] as const;
export type SettingsPageId = (typeof settingsPages)[number]["id"];
