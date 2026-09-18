import type { ComponentType, ReactNode } from "react";
import type { StudioClient } from "@studio/client";
import type { ObjectTarget } from "@studio/client";
import type {
  Asset,
  AssetKey,
  Source,
  ScopeRef,
  ScopeOperation,
  QueryResult,
  QuerySpec,
  Job,
} from "@studio/contracts";

export type BrowseScope =
  | { kind: "all" }
  | { kind: "source" | "collection" | "result"; id: string; name: string }
  | { kind: "selection"; name: string };
export type BrowserPosition = {
  scopeKey: string;
  cursor: string | null;
  pageNumber: number;
  pageSize: number;
  anchor: AssetKey | null;
  version: string | null;
  history?: BrowserHistory;
  scrollTop?: number;
};
export type BrowserHistory = {
  cursors: (string | null)[];
  index: number;
  firstPage: number;
};
export type RankedBrowseSettings = {
  sourceScopeKey?: string;
  views?: Record<string, Omit<RankedBrowseSettings, "views">>;
  scopeKey: string;
  sort: "saved" | "main" | "rescue" | "input" | "direct" | "fused" | "off";
  descending: boolean;
  startPostId: string | null;
  startRank?: string | null;
  startRating?: string | null;
  startCursor: string | null;
};
export type ModuleScopeOption = {
  value: string;
  label: string;
  scope: ScopeRef;
  count: number | null;
};
export type BrowseViewProps = {
  order: QuerySpec["order"];
  onOrder: (order: QuerySpec["order"]) => void;
  rankedBrowse: RankedBrowseSettings | null;
  onRankedBrowse: (settings: RankedBrowseSettings) => void;
  scope: BrowseScope;
  focus: Asset | null;
  focusPending: boolean;
  onFocus: (asset: Asset) => void;
  onInspect: (asset: Asset) => void;
  onScope: (scope: BrowseScope) => void;
  onPick: (keys: AssetKey[], remove?: boolean) => void;
  onScopeOperation: (operation: ScopeOperation) => void;
  selectionRevision: number;
  busy: boolean;
  view: "grid" | "image";
  setView: (view: "grid" | "image") => void;
  position: BrowserPosition | null;
  onPosition: (position: BrowserPosition) => void;
  thumbnailSize: number;
  onThumbnailSize: (size: number) => void;
};
export type ModuleContext = {
  client: StudioClient;
  projectId: string;
  sources: Source[];
  inputOptions: ModuleScopeOption[];
  defaultInput: string;
  browser: BrowseViewProps;
  onResult: (result: QueryResult, name: string) => void;
  onSelect: (result: QueryResult, operation: ScopeOperation) => void;
  onJob: (job: Job, options?: { revealTasks?: boolean }) => void;
  activateView: (id: string, args?: Record<string, string>) => void;
  openPanel: (id: string) => void;
  togglePanel: (id: string) => void;
  closePanel: (id: string) => void;
  panels: string[];
  panelHeight: (id: string) => number;
  resizePanel: (id: string, height: number) => void;
  inspector?: {
    visible: boolean;
    width: number;
    setVisible: (visible: boolean) => void;
    resize: (width: number) => void;
  };
  management?: {
    tab: "properties" | "management";
    open: (
      target: ObjectTarget,
      mode?: "details" | "rename" | "remove",
    ) => void;
    showProperties: () => void;
    header: ReactNode;
    content: ReactNode;
  };
  invocation: { sequence: number; args: Record<string, string> } | null;
};
export type ModuleContribution =
  | {
      kind: "entry";
      id: string;
      label: string;
      icon:
        | "images"
        | "search"
        | "calculator"
        | "archive"
        | "database"
        | "sparkles";
      command: string;
    }
  | {
      kind: "view" | "panel";
      id: string;
      ownsInspector?: boolean;
      load: () => Promise<{ default: ComponentType<ModuleContext> }>;
    }
  | {
      kind: "command";
      id: string;
      execute: (context: ModuleContext, args?: Record<string, string>) => void;
    };
export type ModuleDefinition = {
  id: string;
  version: number;
  protocolVersion: 1;
  draftSchema: { version: number } | null;
  contributions: ModuleContribution[];
};
export class ModuleRegistry {
  readonly modules: ModuleDefinition[] = [];
  private ids = new Set<string>();
  register(module: ModuleDefinition) {
    if (
      !/^[a-z][a-z0-9._-]{1,119}$/.test(module.id) ||
      this.modules.some((m) => m.id === module.id)
    )
      throw new Error("功能模块身份无效或重复：" + module.id);
    if (
      module.protocolVersion !== 1 ||
      module.version < 1 ||
      (module.draftSchema && module.draftSchema.version !== 1)
    )
      throw new Error("功能模块或草稿模式版本不兼容：" + module.id);
    const incoming = new Set<string>();
    for (const c of module.contributions) {
      if (!["entry", "view", "panel", "command"].includes(c.kind))
        throw new Error("未知模块贡献类型");
      const key = c.kind + ":" + c.id;
      if (!c.id || this.ids.has(key) || incoming.has(key))
        throw new Error("模块贡献身份重复：" + key);
      incoming.add(key);
    }
    for (const id of incoming) this.ids.add(id);
    this.modules.push(module);
  }
  validate() {
    for (const entry of this.entries())
      if (!this.commands().some((c) => c.id === entry.command))
        throw new Error("模块入口引用未注册命令：" + entry.command);
  }
  entries() {
    return this.modules.flatMap((m) =>
      m.contributions.filter((c) => c.kind === "entry"),
    );
  }
  commands() {
    return this.modules.flatMap((m) =>
      m.contributions.filter((c) => c.kind === "command"),
    );
  }
  surfaces() {
    return this.modules.flatMap((m) =>
      m.contributions.filter(
        (c): c is Extract<ModuleContribution, { kind: "view" | "panel" }> =>
          c.kind === "view" || c.kind === "panel",
      ),
    );
  }
  execute(id: string, context: ModuleContext, args?: Record<string, string>) {
    const command = this.commands().find((c) => c.id === id);
    if (!command) throw new Error("模块命令尚未注册：" + id);
    command.execute(context, args);
  }
}
