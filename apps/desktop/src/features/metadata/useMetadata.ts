import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { StudioError, assetIdentity } from "@studio/client";
import type { StudioClient } from "@studio/client";
import type { AssetKey, AssetRecord, RankingInput } from "@studio/contracts";

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
  preferred?: RankingInput,
  retainedVersion?: string,
) {
  const [followScore, setFollowScore] = useState(true);
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
    retainedVersion,
    epoch,
  ];
  const overview = useQuery({
    ...queryPolicy,
    queryKey: [...prefix, "records", recordCursor],
    queryFn: ({ signal }) =>
      client.metadata(projectId, key, {
        signal,
        ...(retainedVersion ? { version: retainedVersion } : {}),
        ...(recordCursor ? { cursor: recordCursor } : {}),
      }),
  });
  const data = overview.isError ? undefined : overview.data;
  const preferredRecord: AssetRecord | undefined =
    followScore && preferred?.record_id
      ? {
          record_id: preferred.record_id,
          origin_observation_id: preferred.observation_id ?? null,
          post_id: preferred.post_id ?? null,
          source_md5: null,
          storage_profile: null,
        }
      : undefined;
  const record =
    data?.records.find(
      (r) => r.record_id === (recordId ?? preferredRecord?.record_id),
    ) ??
    preferredRecord ??
    data?.records[0];
  const focused =
    followScore && preferred?.record_id === record?.record_id
      ? preferred?.observation_id
      : undefined;
  const version = data?.version.token;
  const observations = useQuery({
    ...queryPolicy,
    queryKey: [
      ...prefix,
      "observations",
      version,
      record?.record_id,
      observationCursor,
      focused,
    ],
    enabled: !!record && !!version,
    queryFn: ({ signal }) =>
      client.observations(projectId, key, record!.record_id, {
        signal,
        version: version!,
        ...(focused ? { observationId: focused } : {}),
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
    focused,
    browseHistory() {
      setFollowScore(false);
      setRecordId(record?.record_id);
      clearObservation();
    },
    selectRecord(id: string) {
      setFollowScore(false);
      setRecordId(id);
      clearObservation();
    },
    selectObservation(id: string) {
      setObservationId(id);
      setRawRequested(false);
    },
    pageRecords(cursor?: string) {
      setFollowScore(false);
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
      setFollowScore(true);
      setEpoch((v) => v + 1);
      setRecordCursor(undefined);
      setRecordId(undefined);
      clearObservation();
    },
  };
}
