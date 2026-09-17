import { queryOptions } from "@tanstack/react-query";
import type { StudioClient } from "@studio/client";

export function systemPromptsQuery(client: StudioClient) {
  return queryOptions({
    queryKey: [
      "settings",
      "llm",
      "system-prompts",
      client.connection.instance_id,
    ],
    queryFn: ({ signal }) => client.llm.systemPrompts.list(signal),
    // Configuration may be edited from another window; refresh on entry without replacing drafts.
    staleTime: 0,
    refetchOnMount: "always",
  });
}
