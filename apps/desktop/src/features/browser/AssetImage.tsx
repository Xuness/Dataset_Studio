import { useEffect, useRef, useState } from "react";
import { ImageOff, LoaderCircle } from "lucide-react";
import type { Asset } from "@studio/contracts";
import type { StudioClient } from "@studio/client";
export function AssetImage({
  client,
  projectId,
  asset,
  edge = 360,
  className = "",
}: {
  client: StudioClient;
  projectId: string;
  asset: Asset;
  edge?: number;
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [url, setUrl] = useState<string>();
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    let started = false;
    let release: (() => void) | undefined;
    setUrl(undefined);
    setError(null);
    const load = () => {
      if (started) return;
      started = true;
      void client
        .acquireMedia(projectId, asset, edge)
        .then((value) => {
          if (live) {
            release = value.release;
            setUrl(value.url);
          } else value.release();
        })
        .catch((error: unknown) => {
          if (live)
            setError(error instanceof Error ? error.message : "预览暂不可用");
        });
    };
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          load();
          observer.disconnect();
        }
      },
      { rootMargin: "160px" },
    );
    if (ref.current) observer.observe(ref.current);
    return () => {
      live = false;
      observer.disconnect();
      release?.();
    };
  }, [client, projectId, asset.key.source_id, asset.key.asset_id, edge, asset]);
  return (
    <div ref={ref} className={"asset-image " + className}>
      {url ? (
        <img src={url} alt={asset.name} draggable={false} />
      ) : error ? (
        <span title={error}>
          <ImageOff size={24} />
        </span>
      ) : (
        <LoaderCircle className="loading-icon" size={19} />
      )}
    </div>
  );
}
