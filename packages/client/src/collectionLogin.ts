import type { Schema } from "@studio/contracts";

/** A host capability. Implementations own the browser; the UI never sees cookies. */
export interface CollectionLoginAssistant {
  status(): Promise<Schema["CollectionLoginStatus"]>;
  start(
    input: Schema["StartCollectionLogin"],
  ): Promise<Schema["CollectionLoginSession"]>;
  show(id: string): Promise<void>;
  finish(id: string): Promise<Schema["CollectionAccountProbe"]>;
  cancel(id: string): Promise<void>;
}
