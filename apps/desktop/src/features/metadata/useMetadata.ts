import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { StudioError, assetIdentity } from "@studio/client";
import type { StudioClient } from "@studio/client";
import type { AssetKey } from "@studio/contracts";

const queryPolicy = {
  retry: (attempt: number, error: Error) =>
    error instanceof StudioError &&
    (error.code === "SOURCE_INDEX_PREPARING" ||
      (attempt < 2 && error.code === "SOURCE_BUSY")),
  retryDelay: 800,
  staleTime: 15_000,
  gcTime: 30_000,
  refetchOnWindowFocus: false,
};

// This state contains inspection choices only. It has no selection mutation or job port.
export function useMetadata(
  client: StudioClient,
  projectId: string,
  key: AssetKey,
) {
  const [epoch, setEpoch] = useState(0);
  const [recordCursor, setRecordCursor] = useState<string>();
  const [recordId, setRecordId] = useState<string>();
  const [observationCursor, setObservationCursor] = useState<string>();
  const [observationId, setObservationId] = useState<string>();
  const [rawRequested, setRawRequested] = useState(false);
  const prefix = [
    "metadata",
    client.connection.instance_id,
    projectId,
    assetIdentity(key),
    epoch,
  ];
  const overview = useQuery({
    ...queryPolicy,
    queryKey: [...prefix, "records", recordCursor],
    queryFn: ({ signal }) =>
      client.metadata(projectId, key, {
        signal,
        ...(recordCursor ? { cursor: recordCursor } : {}),
      }),
  });
  const data = overview.isError ? undefined : overview.data;
  const record =
    data?.records.find((r) => r.record_id === recordId) ?? data?.records[0];
  const version = data?.version.token;
  const observations = useQuery({
    ...queryPolicy,
    queryKey: [
      ...prefix,
      "observations",
      version,
      record?.record_id,
      observationCursor,
    ],
    enabled: !!record && !!version,
    queryFn: ({ signal }) =>
      client.observations(projectId, key, record!.record_id, {
        signal,
        version: version!,
        ...(observationCursor ? { cursor: observationCursor } : {}),
      }),
  });
  const observationData = observations.isError ? undefined : observations.data;
  const observation =
    observationData?.items.find((o) => o.observation_id === observationId) ??
    observationData?.items.find((o) => o.relation === "asset_origin") ??
    observationData?.items[0];
  const raw = useQuery({
    ...queryPolicy,
    queryKey: [
      ...prefix,
      "raw",
      version,
      record?.record_id,
      observation?.observation_id,
    ],
    enabled: rawRequested && !!record && !!observation && !!version,
    queryFn: ({ signal }) =>
      client.rawMetadata(
        projectId,
        key,
        record!.record_id,
        observation!.observation_id,
        version!,
        signal,
      ),
  });
  function clearObservation() {
    setObservationId(undefined);
    setObservationCursor(undefined);
    setRawRequested(false);
  }
  return {
    overview,
    observations,
    raw,
    data,
    record,
    observation,
    rawRequested,
    recordCursor,
    observationCursor,
    selectRecord(id: string) {
      setRecordId(id);
      clearObservation();
    },
    selectObservation(id: string) {
      setObservationId(id);
      setRawRequested(false);
    },
    pageRecords(cursor?: string) {
      setRecordCursor(cursor);
      setRecordId(undefined);
      clearObservation();
    },
    pageObservations(cursor?: string) {
      setObservationCursor(cursor);
      setObservationId(undefined);
      setRawRequested(false);
    },
    requestRaw() {
      setRawRequested(true);
    },
    refresh() {
      setEpoch((v) => v + 1);
      setRecordCursor(undefined);
      setRecordId(undefined);
      clearObservation();
    },
  };
}
