import { useEffect, useRef, useState } from "react";
import { ImageOff, LoaderCircle } from "lucide-react";
import type { Asset } from "@studio/contracts";
import type { StudioClient } from "@studio/client";
import "./media.css";
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
  const [offline, setOffline] = useState(false);
  const [verifiedMs, setVerifiedMs] = useState(0);
  useEffect(() => {
    let live = true;
    let started = false;
    let release: (() => void) | undefined;
    const abort = new AbortController();
    setUrl(undefined);
    setError(null);
    setOffline(false);
    setVerifiedMs(0);
    const load = () => {
      if (started) return;
      started = true;
      void client
        .acquireMedia(projectId, asset, edge, { signal: abort.signal })
        .then((value) => {
          if (live) {
            release = value.release;
            setUrl(value.url);
            setOffline(value.offline);
            setVerifiedMs(value.verifiedMs);
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
      { rootMargin: "0px" },
    );
    if (ref.current) observer.observe(ref.current);
    return () => {
      live = false;
      abort.abort();
      observer.disconnect();
      release?.();
    };
  }, [client, projectId, asset.key.source_id, asset.key.asset_id, edge, asset]);
  return (
    <div
      ref={ref}
      className={"asset-image " + className}
      title={
        verifiedMs
          ? `来源上次验证：${new Date(verifiedMs).toLocaleString()}${offline ? "；当前使用离线缓存" : ""}`
          : undefined
      }
    >
      {url ? (
        <>
          <img src={url} alt={asset.name} draggable={false} />
          {offline && (
            <span
              className="offline-cache-badge"
              title="来源离线，显示此前校验过的缩略图；目前无法重新验证来源。"
            >
              离线缓存
            </span>
          )}
        </>
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
