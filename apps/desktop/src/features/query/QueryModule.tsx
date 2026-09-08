import type { ModuleContext } from "@studio/ui";
import { QueryPanel } from "./QueryPanel.js";
import { useProjectQueries } from "./useProjectQueries.js";
export default function QueryModule(context: ModuleContext) {
  const model = useProjectQueries(context.client, context.projectId);
  return (
    <QueryPanel
      client={context.client}
      projectId={context.projectId}
      sources={context.sources}
      model={model}
      onResult={context.onResult}
      onSelect={context.onSelect}
      onClose={() => context.closePanel("core.query")}
      browserScope={context.browser.scope}
      inputOptions={context.inputOptions}
      height={context.panelHeight("core.query")}
      onHeight={(height) => context.resizePanel("core.query", height)}
    />
  );
}
