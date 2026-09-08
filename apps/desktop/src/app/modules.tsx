import { lazy } from "react";
import { ModuleRegistry } from "@studio/ui";
const registry = new ModuleRegistry();
registry.register({
  id: "core.browser",
  version: 1,
  protocolVersion: 1,
  draftSchema: { version: 1 },
  contributions: [
    {
      kind: "entry",
      id: "browser",
      label: "资料浏览",
      icon: "images",
      command: "browser.open",
    },
    {
      kind: "view",
      id: "core.browser",
      load: () => import("../features/browser/Browser.js"),
    },
    {
      kind: "command",
      id: "browser.open",
      execute: (context) => context.activateView("core.browser"),
    },
  ],
});
registry.register({
  id: "core.query",
  version: 1,
  protocolVersion: 1,
  draftSchema: { version: 1 },
  contributions: [
    {
      kind: "entry",
      id: "query",
      label: "项目查询",
      icon: "search",
      command: "query.open",
    },
    {
      kind: "panel",
      id: "core.query",
      load: () => import("../features/query/QueryModule.js"),
    },
    {
      kind: "command",
      id: "query.open",
      execute: (context) => context.openPanel("core.query"),
    },
  ],
});
registry.register({
  id: "core.tools",
  version: 1,
  protocolVersion: 1,
  draftSchema: { version: 1 },
  contributions: [
    {
      kind: "entry",
      id: "tools",
      label: "计算工具",
      icon: "calculator",
      command: "tools.open",
    },
    {
      kind: "view",
      id: "core.tools",
      load: () => import("../features/tools/ToolPanel.js"),
    },
    {
      kind: "command",
      id: "tools.open",
      execute: (context, args) => context.activateView("core.tools", args),
    },
  ],
});
registry.register({
  id: "core.artifacts",
  version: 1,
  protocolVersion: 1,
  draftSchema: { version: 1 },
  contributions: [
    {
      kind: "entry",
      id: "artifacts",
      label: "项目成果",
      icon: "archive",
      command: "artifacts.open",
    },
    {
      kind: "view",
      id: "core.artifacts",
      load: () => import("../features/artifacts/ArtifactPanel.js"),
    },
    {
      kind: "command",
      id: "artifacts.open",
      execute: (context) => context.activateView("core.artifacts"),
    },
  ],
});
registry.register({
  id: "core.resources",
  version: 1,
  protocolVersion: 1,
  draftSchema: { version: 1 },
  contributions: [
    {
      kind: "view",
      id: "core.resources",
      load: () => import("../features/resources/ResourcePanel.js"),
    },
    {
      kind: "command",
      id: "resources.open",
      execute: (context) => context.activateView("core.resources"),
    },
  ],
});
registry.validate();
export const modules = registry;
export const moduleViews = new Map(
  registry
    .surfaces()
    .map((surface) => [
      surface.id,
      { kind: surface.kind, Component: lazy(surface.load) },
    ]),
);
