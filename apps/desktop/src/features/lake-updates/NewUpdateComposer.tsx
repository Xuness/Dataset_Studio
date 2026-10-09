import { useState } from "react";
import type { StudioClient } from "@studio/client";
import type { Schema } from "@studio/contracts";
import { CollectionComposer } from "./CollectionComposer.js";
import { UpdateComposer } from "./UpdateComposer.js";
import { PinterestComposer } from "./PinterestComposer.js";
import type { CollectionDefinition, WorkspaceLake } from "./collectionModel.js";
import type { Lake } from "./model.js";

export const updateLakes = (lakes: WorkspaceLake[]): Lake[] =>
  lakes
    .filter((l) => ["danbooru", "gelbooru", "yandere"].includes(l.site))
    .map((l) => ({ ...l, site: l.site as Lake["site"] }));
export function NewUpdateComposer({
  client,
  lakes,
  initialLake,
  preset,
  capabilities,
  onClose,
  onCreated,
}: {
  client: StudioClient;
  lakes: WorkspaceLake[];
  initialLake: string;
  preset?: CollectionDefinition | undefined;
  capabilities: Schema["LakeUpdateCapability"][];
  onClose: () => void;
  onCreated: (
    id: string,
    kind: "job" | "schedule",
    family: "update" | "collection" | "pinterest",
  ) => void;
}) {
  const initialSite =
    lakes.find((l) => l.id === initialLake)?.site ?? lakes[0]?.site;
  const [family, setFamily] = useState(
    preset
      ? "collection"
      : initialSite === "pinterest"
        ? "pinterest"
        : initialSite === "pixiv"
          ? "collection"
          : "update",
  );
  const header = (
    <label className="collection-source-picker">
      来源
      <select
        aria-label="更新来源"
        value={family}
        onChange={(e) => setFamily(e.target.value)}
      >
        <option value="update">Danbooru / Yandere / Gelbooru</option>
        <option value="collection">Pixiv</option>
        <option value="pinterest">Pinterest</option>
      </select>
    </label>
  );
  return family === "pinterest" ? (
    <PinterestComposer
      client={client}
      lakes={lakes.filter((l) => l.site === "pinterest")}
      initialLake={initialLake}
      sourceHeader={header}
      onClose={onClose}
      onCreated={(id, kind) => onCreated(id, kind, "pinterest")}
    />
  ) : family === "collection" ? (
    <CollectionComposer
      client={client}
      lakes={lakes.filter((l) => l.site === "pixiv")}
      initialLake={initialLake}
      preset={preset}
      sourceHeader={header}
      onClose={onClose}
      onCreated={(id, kind) => onCreated(id, kind, "collection")}
    />
  ) : (
    <UpdateComposer
      client={client}
      initialLake={initialLake}
      lakes={updateLakes(lakes)}
      capabilities={capabilities}
      sourceHeader={header}
      onClose={onClose}
      onCreated={(id, kind) => onCreated(id, kind, "update")}
    />
  );
}
