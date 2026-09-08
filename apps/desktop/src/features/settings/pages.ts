import { Database, List, Gauge } from "lucide-react";
import { CacheSettingsPage } from "./CacheSettingsPage.js";
import { CacheManagerPage } from "./CacheManagerPage.js";
import { PerformanceSettingsPage } from "./PerformanceSettingsPage.js";

/** Register a settings page here; it stays outside the project's business views. */
export const settingsPages = [
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
