import { useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { Download, LoaderCircle, Scan } from "lucide-react";
import { assetTitle, ContextMenu, contextMenuAt, errorText } from "@studio/ui";
import type { ContextMenuState, MoreMenuItem } from "@studio/ui";
import type { StudioClient } from "@studio/client";
import type { Asset } from "@studio/contracts";
import { AssetImage } from "./AssetImage.js";
import { useImageActions } from "./originals.js";
import {
  maxZoom,
  useImageZoom,
  ZoomableImage,
  ZoomControls,
} from "./ZoomableImage.js";

export const viewerHint =
  "← → 切换 · 滚轮缩放 · 双击放大 · 拖动平移 · 1 原始尺寸 · 0 适应 · Ctrl+C 复制 · Ctrl+S 另存原图 · 右键更多 ";

type Original =
  | { key: string; state: "loading" }
  | { key: string; state: "ready"; url: string }
  | { key: string; state: "error"; error: string };

/**
 * One large-image viewer for browse and ranking pages: a fitted preview that
 * switches to the stored original once the user zooms in or asks for 1:1.
 */
export function useImageViewer(
  client: StudioClient,
  projectId: string,
  asset: Asset | null,
  /** Extra zoom reset key, e.g. the page view mode. */
  resetKey = "",
) {
  const identity = asset ? asset.key.source_id + ":" + asset.key.asset_id : "";
  const zoom = useImageZoom(resetKey + ":" + identity);
  const actions = useImageActions(client, projectId);
  const [actualFor, setActualFor] = useState("");
  const [original, setOriginal] = useState<Original | null>(null);
  const holder = useRef<{ key: string; abort?: AbortController; url?: string }>(
    { key: "" },
  );
  const wanted = zoom.zoom > 1 || actualFor === identity;
  useEffect(
    () => () => {
      holder.current.abort?.abort();
      if (holder.current.url) URL.revokeObjectURL(holder.current.url);
      holder.current = { key: "" };
    },
    [identity],
  );
  useEffect(() => {
    if (!asset || !wanted || holder.current.key === identity) return;
    const abort = new AbortController();
    holder.current = { key: identity, abort };
    setOriginal({ key: identity, state: "loading" });
    client.original(projectId, asset.key, abort.signal).then(
      (blob) => {
        if (abort.signal.aborted) return;
        const url = URL.createObjectURL(blob);
        holder.current.url = url;
        setOriginal({ key: identity, state: "ready", url });
      },
      (error: unknown) => {
        if (!abort.signal.aborted)
          setOriginal({
            key: identity,
            state: "error",
            error: errorText(error),
          });
      },
    );
  }, [client, projectId, asset, identity, wanted]);
  const current = original?.key === identity ? original : null;
  const requestActual = useCallback(() => setActualFor(identity), [identity]);
  const clearActual = useCallback(() => setActualFor(""), []);
  return {
    asset,
    identity,
    zoom,
    actions,
    original: current,
    /** 1:1 is applied by the stage once the original's size is known. */
    actualPending: actualFor === identity,
    requestActual,
    clearActual,
    /** Zoom keys plus Ctrl+C (copy image) and Ctrl+S (save original). */
    onKey(event: KeyboardEvent) {
      if (!asset) return false;
      const key = event.key.toLowerCase();
      if ((event.ctrlKey || event.metaKey) && !event.altKey) {
        if (key === "c" && document.getSelection()?.isCollapsed !== false) {
          event.preventDefault();
          void actions.copy(asset);
          return true;
        }
        if (key === "s") {
          event.preventDefault();
          void actions.save(asset);
          return true;
        }
        return false;
      }
      if (key === "1" && !event.altKey && !event.shiftKey) {
        event.preventDefault();
        setActualFor(identity);
        return true;
      }
      return zoom.onKey(event);
    },
  };
}
export type ImageViewer = ReturnType<typeof useImageViewer>;

export function ImageStage({
  viewer,
  client,
  projectId,
  edge,
  menuItems = [],
}: {
  viewer: ImageViewer;
  client: StudioClient;
  projectId: string;
  edge: number;
  /** Page-specific entries appended after the shared image actions. */
  menuItems?: MoreMenuItem[];
}) {
  const { asset, zoom, original, actions } = viewer;
  const image = useRef<HTMLImageElement>(null);
  const [menu, setMenu] = useState<ContextMenuState | null>(null);
  const { actualPending, clearActual } = viewer;
  const setZoom = zoom.set;
  useEffect(() => {
    const element = image.current;
    if (!actualPending || original?.state !== "ready" || !element) return;
    const fit = () => {
      const box = element.closest(".zoom-viewport");
      if (!box || !element.naturalWidth) return;
      const scale = Math.min(
        box.clientWidth / element.naturalWidth,
        box.clientHeight / element.naturalHeight,
      );
      setZoom(scale > 0 ? 1 / scale : 1);
      clearActual();
    };
    if (element.complete) fit();
    else element.addEventListener("load", fit, { once: true });
    return () => element.removeEventListener("load", fit);
  }, [actualPending, original, setZoom, clearActual]);
  if (!asset) return null;
  return (
    <>
      <ZoomableImage
        zoom={zoom}
        onContextMenu={(event) =>
          setMenu(
            contextMenuAt(event, assetTitle(asset), [
              {
                label: "另存原图…",
                shortcut: "Ctrl+S",
                action: () => void actions.save(asset),
              },
              {
                label: "复制图像",
                shortcut: "Ctrl+C",
                action: () => void actions.copy(asset),
              },
              {
                label: "复制图像身份",
                action: () => actions.copyIdentity(asset),
              },
              {
                label: "原始尺寸 1:1",
                shortcut: "1",
                separator: true,
                action: viewer.requestActual,
              },
              {
                label: "适应窗口",
                shortcut: "0",
                disabled: zoom.zoom === 1,
                action: zoom.reset,
              },
              ...menuItems.map((item, index) =>
                index === 0 ? { ...item, separator: true } : item,
              ),
            ]),
          )
        }
        overlay={
          original?.state === "loading" ? (
            <span className="original-status" role="status">
              <LoaderCircle className="loading-icon" size={12} />
              正在载入原图…
            </span>
          ) : original?.state === "error" ? (
            <span className="original-status" data-tone="error" role="status">
              原图不可用，显示预览：{original.error}
            </span>
          ) : null
        }
      >
        {original?.state === "ready" ? (
          <div className="asset-image original-image">
            <img
              ref={image}
              src={original.url}
              alt={asset.name}
              draggable={false}
            />
          </div>
        ) : (
          <AssetImage
            client={client}
            projectId={projectId}
            asset={asset}
            edge={edge}
          />
        )}
      </ZoomableImage>
      <ContextMenu state={menu} onClose={() => setMenu(null)} />
    </>
  );
}

/** Zoom, 1:1 and save buttons for a viewer toolbar. */
export function ImageTools({ viewer }: { viewer: ImageViewer }) {
  const { asset, zoom, original } = viewer;
  return (
    <>
      <ZoomControls zoom={zoom} />
      <button
        type="button"
        aria-label="原始尺寸"
        title="原始尺寸 1:1（1）"
        disabled={!asset || zoom.zoom >= maxZoom}
        onClick={viewer.requestActual}
      >
        <Scan size={15} />
        1:1
      </button>
      <span
        className="original-badge"
        data-active={original?.state === "ready" || undefined}
        title={
          original?.state === "ready"
            ? "正在显示原图"
            : "放大或 1:1 时载入原图；适应窗口时显示预览"
        }
      >
        {original?.state === "ready" ? "原图" : "预览"}
      </span>
      <button
        type="button"
        aria-label="另存原图"
        title="另存原图（Ctrl+S）"
        disabled={!asset}
        onClick={() => asset && void viewer.actions.save(asset)}
      >
        <Download size={15} />
      </button>
    </>
  );
}
