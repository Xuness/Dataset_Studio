export interface paths {
    "/v1/cache/rating-bases": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_rating_bases"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/cache/rating-bases/{source_id}/cancel": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["cancel_rating_basis_build"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/cache/rating-bases/{source_id}/{rating}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put: operations["set_rating_basis_retention"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/cache/rating-bases/{source_id}/{rating}/release": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["release_rating_basis"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/health": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["health"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/operators": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["operators"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/preferences/{key}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["preference"];
        put: operations["save_preference"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["projects"];
        put?: never;
        post: operations["create_project"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/open": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["open_project"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["project"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/artifacts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["artifacts"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/artifacts/{artifact_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["get_artifact"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/artifacts/{artifact_id}/ranking": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["ranking_summary"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/artifacts/{artifact_id}/ranking/evidence": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["ranking_evidence"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/artifacts/{artifact_id}/ranking/rows": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["ranking_rows"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/artifacts/{artifact_id}/ranking/worksets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["ranking_workset"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/artifacts/{artifact_id}/release": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["release_artifact"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/artifacts/{artifact_id}/rows": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["rows"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/artifacts/{artifact_id}/verify": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["verify"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/assets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["assets"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/cache-entries": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_cache_entries"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/cache-session": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["keep_cache_session"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/close": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["close_project"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/collections": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["collections"];
        put?: never;
        post: operations["create_collection"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/drafts/{module_id}/{instance_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["draft"];
        put: operations["save_draft"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/events": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["events"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/jobs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list_jobs"];
        put?: never;
        post: operations["submit_job"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/jobs/{job_id}/artifact": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["artifact"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/jobs/{job_id}/cancel": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["cancel_job"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/jobs/{job_id}/ranking": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["ranking_job_result"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/jobs/{job_id}/retry": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["retry"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/jobs/{job_id}/run": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["run"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/open": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["open_recent"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/queries": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["definitions"];
        put?: never;
        post: operations["create_definition"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/queries/{query_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["definition"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch: operations["update_definition"];
        trace?: never;
    };
    "/v1/projects/{project_id}/queries/{query_id}/results": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["build"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/query-results": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["results"];
        put?: never;
        post: operations["run_query"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/query-results/{result_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["result"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/query-results/{result_id}/assets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["result_assets"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/query-results/{result_id}/cache-release": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["release_cache_entry"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/query-results/{result_id}/cancel": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["cancel"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/query-results/{result_id}/leases/{lease_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["lease_result"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/query-results/{result_id}/leases/{lease_id}/release": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["release_result_lease"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/query-results/{result_id}/release": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["release"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/query-results/{result_id}/retention": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put: operations["set_result_retention"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/query-results/{result_id}/validity": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["validity"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/read-requests/{request_id}/cancel": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["cancel_read_subscription"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/scopes/capture": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["capture"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/selection": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["selection"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch: operations["change_selection"];
        trace?: never;
    };
    "/v1/projects/{project_id}/selection/scope": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["select_scope"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/sources": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["sources"];
        put?: never;
        post: operations["attach_source"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["asset_detail"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/media": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["media"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/metadata": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["metadata"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/records/{record_id}/observations": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["observations"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/sources/{source_id}/assets/{asset_id}/records/{record_id}/observations/{observation_id}/raw": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["raw_metadata"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/sources/{source_id}/fields": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["fields"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/sources/{source_id}/rating-bases": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["prebuild_rating_bases"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/sources/{source_id}/relink": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["relink"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/tools/jobs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["submit"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/tools/validate-scope": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["validate_scope"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/resources": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["status"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/resources/cache": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put: operations["configure"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/resources/cache/clear": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["clear"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/resources/query": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put: operations["configure_query"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/resources/query-cache": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put: operations["configure_query_cache"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/resources/query-cache/clear": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["clear_query_cache"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/settings": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["read_settings"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/settings/cache": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put: operations["configure_cache_settings"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/settings/cache/clear": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["clear_cache_tier"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/shutdown": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["shutdown"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        ApiError: {
            code: string;
            message: string;
            request_id: string;
        };
        Artifact: {
            /** Format: int64 */
            count?: number | null;
            created_at: string;
            files: components["schemas"]["ArtifactFile"][];
            id: string;
            issue?: string | null;
            job_id: string;
            kind: string;
            name: string;
            output_id: string;
            project_id: string;
            provenance: components["schemas"]["ArtifactProvenance"];
            /** Format: int32 */
            schema_version: number;
            state: components["schemas"]["ArtifactState"];
        };
        ArtifactFile: {
            bytes?: string | null;
            media_type: string;
            path: string;
            sha256?: string | null;
        };
        ArtifactPage: {
            artifact_id: string;
            items: components["schemas"]["ArtifactRow"][];
            next_cursor?: string | null;
        };
        ArtifactProvenance: {
            /** Format: int32 */
            attempt?: number | null;
            evidence: string;
            fields_frozen: boolean;
            input_artifacts: string[];
            input_scope?: null | components["schemas"]["ScopeRef"];
            input_sha256?: string | null;
            run?: null | components["schemas"]["OperatorRun"];
        };
        ArtifactRow: {
            data: unknown;
            key: components["schemas"]["AssetKey"];
            /** Format: int64 */
            ordinal: number;
            scalar?: null | components["schemas"]["ScalarValue"];
        };
        /** @enum {string} */
        ArtifactState: "legacy" | "publishing" | "ready" | "released" | "unavailable";
        Artifacts: {
            items: components["schemas"]["Artifact"][];
            next_cursor?: string | null;
        };
        Asset: {
            bytes: string;
            extension: string;
            key: components["schemas"]["AssetKey"];
            name: string;
            selected: boolean;
            source_name: string;
            summary?: null | components["schemas"]["AssetSummary"];
        };
        AssetKey: {
            asset_id: string;
            source_id: string;
        };
        AssetPage: {
            items: components["schemas"]["Asset"][];
            next_cursor?: string | null;
            preparing?: string | null;
            result_id?: string | null;
            revision: string;
        };
        AssetRecord: {
            origin_observation_id?: string | null;
            post_id?: string | null;
            record_id: string;
            source_md5?: string | null;
            storage_profile?: string | null;
        };
        AssetSummary: {
            issue?: string | null;
            post_count?: string | null;
            post_ids: string[];
            /** @description available, unlinked, unavailable, or unsupported; unavailable is not unlinked. */
            status: string;
            version?: string | null;
        };
        AttachSource: {
            index_root?: string | null;
            kind: string;
            media_root?: string | null;
            name: string;
        };
        BuildQuery: {
            /** Format: int64 */
            expected_revision: number;
        };
        CacheEntries: {
            items: components["schemas"]["CacheEntry"][];
            next_cursor?: string | null;
        };
        CacheEntry: {
            estimated_bytes: string;
            family_id: string;
            fixed: boolean;
            last_used_millis: string;
            /** Format: int64 */
            members: number;
            project_id: string;
            /** Format: int64 */
            protected_results: number;
            result_id: string;
            session_only: boolean;
            spec: components["schemas"]["QuerySpec"];
            tier: string;
        };
        CacheSettings: {
            /** Format: int32 */
            long_term_idle_days?: number | null;
            /** Format: int32 */
            long_term_mib: number;
            /** Format: int32 */
            preview_mib: number;
            /** Format: int32 */
            temporary_idle_hours: number;
            /** Format: int32 */
            temporary_mib: number;
            temporary_session_only: boolean;
            /** Format: int32 */
            total_mib: number;
        };
        CacheStorageOverview: {
            /** Format: int64 */
            active_views: number;
            cleanup_pending: boolean;
            fixed_member_bytes: string;
            long_term_bytes: string;
            /** Format: int64 */
            long_term_results: number;
            preview_bytes: string;
            project_member_bytes: string;
            /** Format: int64 */
            protected_results: number;
            rating_basis_bytes: string;
            source_index_bytes: string;
            temporary_bytes: string;
            /** Format: int64 */
            temporary_results: number;
            total_bytes: string;
            total_quota_bytes: string;
            working_temporary_bytes: string;
        };
        CaptureScope: {
            scope: components["schemas"]["ScopeRef"];
        };
        ChangeSelection: {
            add: components["schemas"]["AssetKey"][];
            clear: boolean;
            /** Format: int64 */
            expected_revision: number;
            remove: components["schemas"]["AssetKey"][];
        };
        ChangeSelectionScope: {
            /** Format: int64 */
            expected_revision: number;
            operation: components["schemas"]["ScopeOperation"];
            scope: components["schemas"]["ScopeRef"];
        };
        ClearCacheTier: {
            tier?: string | null;
        };
        Collection: {
            /** Format: int64 */
            count: number;
            id: string;
            name: string;
        };
        Collections: {
            items: components["schemas"]["Collection"][];
        };
        CreateCollection: {
            name: string;
            scope?: null | components["schemas"]["ScopeRef"];
        };
        CreateProject: {
            name: string;
            parent_directory?: string | null;
        };
        Draft: {
            instance_id: string;
            module_id: string;
            project_id: string;
            /** Format: int64 */
            revision: number;
            /** Format: int32 */
            schema_version: number;
            updated_at: string;
            value: unknown;
        };
        EngineConnection: {
            /** Format: int32 */
            api_version: number;
            endpoint: string;
            instance_id: string;
            /** Format: int32 */
            pid: number;
            token: string;
        };
        FieldDefinition: {
            basis: string;
            cost: string;
            display: boolean;
            field_type: components["schemas"]["FieldType"];
            id: string;
            missing: string;
            name: string;
            operators: components["schemas"]["QueryOperator"][];
            sortable: boolean;
            unit?: string | null;
        };
        FieldDirectory: {
            fields: components["schemas"]["FieldDefinition"][];
            max_conditions: number;
            observation_rules: components["schemas"]["ObservationRule"][];
            orders: components["schemas"]["QueryOrder"][];
            source_id: string;
            /** Format: int32 */
            version: number;
        };
        /** @enum {string} */
        FieldType: "text" | "integer" | "boolean" | "tags";
        Health: {
            /** Format: int32 */
            api_version: number;
            instance_id: string;
            version: string;
        };
        Job: {
            artifact?: string | null;
            /** Format: int32 */
            attempt: number;
            /** Format: int64 */
            completed: number;
            created_at: string;
            error?: string | null;
            id: string;
            input_members_frozen: boolean;
            input_scope?: null | components["schemas"]["ScopeRef"];
            operator: string;
            project_id: string;
            stage?: null | components["schemas"]["JobStage"];
            status: string;
            /** Format: int64 */
            total: number;
        };
        JobRun: {
            fields: components["schemas"]["ScalarInput"][];
            run: components["schemas"]["OperatorRun"];
            source_versions: components["schemas"]["QuerySourceVersion"][];
        };
        JobStage: {
            /** Format: int64 */
            completed: number;
            name: string;
            /** Format: int64 */
            total: number;
        };
        Jobs: {
            items: components["schemas"]["Job"][];
        };
        MaybeDraft: {
            draft?: null | components["schemas"]["Draft"];
        };
        MaybePreference: {
            preference?: null | components["schemas"]["Preference"];
        };
        MetadataField: {
            missing_reason?: string | null;
            name: string;
            provenance: string;
            truncated: boolean;
            value?: null | components["schemas"]["MetadataValue"];
        };
        MetadataObject: {
            bytes: string;
            extension: string;
            key: components["schemas"]["AssetKey"];
            name: string;
            source_name: string;
        };
        MetadataOverview: {
            dimensions_evidence: string;
            next_cursor?: string | null;
            object: components["schemas"]["MetadataObject"];
            records: components["schemas"]["AssetRecord"][];
            /** Format: int32 */
            stored_height?: number | null;
            /** Format: int32 */
            stored_width?: number | null;
            version: components["schemas"]["ReadVersion"];
        };
        MetadataQuery: {
            cursor?: string | null;
            limit?: number | null;
            version?: string | null;
        };
        MetadataValue: {
            /** @enum {string} */
            type: "text";
            value: string;
        } | {
            /** @enum {string} */
            type: "integer";
            value: string;
        } | {
            /** @enum {string} */
            type: "boolean";
            value: boolean;
        } | {
            /** @enum {string} */
            type: "tags";
            value: string[];
        } | {
            /** @enum {string} */
            type: "timestamp";
            value: string;
        };
        Observation: {
            commit_sequence?: string | null;
            fields: components["schemas"]["MetadataField"][];
            ingested_at?: string | null;
            observation_id: string;
            observed_at?: string | null;
            post_id?: string | null;
            relation: string;
            row_id: string;
            source_key?: string | null;
            source_kind?: string | null;
            time_quality?: string | null;
        };
        ObservationPage: {
            items: components["schemas"]["Observation"][];
            next_cursor?: string | null;
            record_id: string;
            version: components["schemas"]["ReadVersion"];
        };
        /** @enum {string} */
        ObservationRule: "current_post" | "any_observation";
        OkResponse: {
            ok: boolean;
        };
        OpenProject: {
            directory: string;
        };
        OperatorCapabilities: {
            cancel: boolean;
            checkpoint: boolean;
            deterministic: boolean;
            item_failures: boolean;
            retry: boolean;
        };
        OperatorDescriptor: {
            capabilities: components["schemas"]["OperatorCapabilities"];
            id: string;
            input_scopes: string[];
            name: string;
            outputs: components["schemas"]["OutputDescriptor"][];
            parameters: components["schemas"]["ParameterDescriptor"][];
            /** Format: int32 */
            parameters_version: number;
            resources: components["schemas"]["ResourceRequirements"];
            /** Format: int32 */
            version: number;
        };
        OperatorRun: {
            operator_id: string;
            /** Format: int32 */
            operator_version: number;
            parameters: unknown;
            /** Format: int32 */
            parameters_version: number;
        };
        Operators: {
            items: components["schemas"]["OperatorDescriptor"][];
            /** Format: int32 */
            protocol_version: number;
        };
        OutputDescriptor: {
            id: string;
            kind: string;
            name: string;
            /** Format: int32 */
            schema_version: number;
            subject: string;
        };
        ParameterDescriptor: {
            default_value: unknown;
            id: string;
            name: string;
            required: boolean;
            value_type: string;
        };
        Preference: {
            key: string;
            /** Format: int64 */
            revision: number;
            /** Format: int32 */
            schema_version: number;
            value: unknown;
        };
        PreviewActivity: {
            active_subscriptions: number;
            /** Format: int64 */
            batches: number;
            /** Format: int64 */
            cancelled_before_read: number;
            /** Format: int64 */
            cancelled_finished: number;
            /** Format: int64 */
            cancelled_last: number;
            /** Format: int64 */
            decode_ms: number;
            /** Format: int64 */
            generated: number;
            max_batch: number;
            /** Format: int64 */
            max_cancel_latency_ms: number;
            /** Format: int64 */
            max_queue_wait_ms: number;
            /** Format: int64 */
            pack_opens: number;
            /** Format: int64 */
            queue_wait_ms: number;
            queued: number;
            /** Format: int64 */
            read_ms: number;
            /** Format: int64 */
            seeks: number;
            /** Format: int64 */
            shared: number;
            source_bytes: string;
        };
        PreviewCacheStatus: {
            bytes: string;
            clear_pending: boolean;
            /** Format: int64 */
            corrupt: number;
            directory: string;
            /** Format: int64 */
            entries: number;
            /** Format: int64 */
            evicted: number;
            /** Format: int64 */
            hits: number;
            index_rebuilt: boolean;
            maintenance_pending: boolean;
            /** Format: int64 */
            maintenance_removed: number;
            /** Format: int64 */
            misses: number;
            pinned: number;
            quota_bytes: string;
            read_bytes: string;
            /** Format: int64 */
            writes: number;
        };
        Project: {
            created_at: string;
            directory: string;
            id: string;
            name: string;
            /** Format: int64 */
            revision: number;
        };
        ProjectClose: {
            project_id: string;
            state: components["schemas"]["ProjectState"];
        };
        ProjectEvent: {
            kind: string;
            project_id: string;
            resource_id: string;
            /** Format: int64 */
            sequence: number;
        };
        /** @enum {string} */
        ProjectState: "closed" | "open" | "background" | "draining" | "unavailable";
        ProjectSummary: {
            directory: string;
            id: string;
            issue?: string | null;
            name: string;
            opened_at: string;
            state: components["schemas"]["ProjectState"];
        };
        Projects: {
            items: components["schemas"]["ProjectSummary"][];
        };
        QueryCacheInfo: {
            basis_ratings: string[];
            /** Format: int64 */
            candidate_records: number;
            /** Format: int64 */
            changed_members: number;
            /** Format: int64 */
            evaluated_objects: number;
            fixed: boolean;
            mode: string;
            session_only: boolean;
            tier: string;
        };
        QueryCacheStatus: {
            /** Format: int64 */
            active_views: number;
            cleanup_pending: boolean;
            database_free_bytes: string;
            /** Format: int64 */
            incremental_results: number;
            /** Format: int32 */
            max_age_days: number;
            /** Format: int64 */
            member_versions: number;
            /** Format: int64 */
            protected_results: number;
            quota_bytes: string;
            /** Format: int64 */
            reclaimed_queries: number;
            result_storage_bytes: string;
            /** Format: int64 */
            retained_queries: number;
            /** Format: int64 */
            reused_results: number;
            source_index_bytes: string;
            /** Format: int64 */
            source_indexes: number;
        };
        QueryCondition: {
            field: string;
            operator: components["schemas"]["QueryOperator"];
            value?: null | components["schemas"]["QueryValue"];
        };
        QueryDefinition: {
            created_at: string;
            id: string;
            name: string;
            project_id: string;
            /** Format: int64 */
            revision: number;
            spec: components["schemas"]["QuerySpec"];
        };
        QueryDefinitions: {
            items: components["schemas"]["QueryDefinition"][];
            next_cursor?: string | null;
        };
        /** @enum {string} */
        QueryOperator: "eq" | "ne" | "gte" | "lte" | "has_tag" | "in" | "has_all_tags" | "has_any_tags" | "has_no_tags" | "is_missing" | "is_present";
        /** @enum {string} */
        QueryOrder: "asset_key_asc" | "asset_key_desc" | "post_id_asc" | "post_id_desc";
        QueryResourceLimits: {
            active_query_memory_bytes?: string | null;
            metadata_memory_bytes: string;
            native_query_memory_bytes: string;
            query_memory_bytes: string;
            result_staging_disk_bytes: string;
            result_work_memory_bytes: string;
            temporary_disk_bytes: string;
        };
        QueryResult: {
            cache: components["schemas"]["QueryCacheInfo"];
            /** Format: int64 */
            count?: number | null;
            created_at: string;
            definition_id?: string | null;
            /** Format: int64 */
            definition_revision?: number | null;
            error?: string | null;
            id: string;
            /** Format: int64 */
            processed: number;
            project_id: string;
            source_versions: components["schemas"]["QuerySourceVersion"][];
            spec: components["schemas"]["QuerySpec"];
            state: components["schemas"]["ResultState"];
        };
        QueryResults: {
            items: components["schemas"]["QueryResult"][];
            next_cursor?: string | null;
        };
        QuerySourceVersion: {
            analysis_sequence?: string | null;
            catalog_revision: string;
            consistency: string;
            source_id: string;
        };
        QuerySpec: {
            conditions: components["schemas"]["QueryCondition"][];
            input_scope?: null | components["schemas"]["ScopeRef"];
            observation_rule: components["schemas"]["ObservationRule"];
            order: components["schemas"]["QueryOrder"];
            source_ids: string[];
            /** Format: int32 */
            version: number;
        };
        QueryValue: {
            /** @enum {string} */
            type: "text";
            value: string;
        } | {
            /** @enum {string} */
            type: "integer";
            value: string;
        } | {
            /** @enum {string} */
            type: "boolean";
            value: boolean;
        } | {
            /** @enum {string} */
            type: "text_list";
            value: string[];
        };
        RankingBasis: {
            /** Format: int32 */
            index: number;
            result_id?: string | null;
            spec: components["schemas"]["QuerySpec"];
        };
        /** @enum {string} */
        RankingEligibility: "eligible" | "metadata_unavailable" | "rating_unknown" | "rating_excluded" | "dimensions_unknown" | "dimensions_excluded" | "policy_excluded" | "duplicate";
        RankingEvidence: {
            bases: components["schemas"]["RankingBasis"][];
            job_run: components["schemas"]["JobRun"];
            metadata_fields: string[];
        };
        RankingFilter: {
            /** @default null */
            eligibility: null | components["schemas"]["RankingEligibility"];
            /** @default false */
            missing_only: boolean;
            /** @default main */
            order: components["schemas"]["RankingOrder"];
            /** @default null */
            rating: string | null;
            /** @default null */
            route: null | components["schemas"]["RankingRoute"];
            /** @default false */
            selected_only: boolean;
            /**
             * Format: int64
             * @default null
             */
            top: number | null;
        };
        RankingInput: {
            artists: string[];
            asset_id: string;
            basis_ids: number[];
            created_at_us?: string | null;
            /** Format: int32 */
            damage_classes: number;
            dimension_basis: string;
            down_score?: string | null;
            /** Format: int64 */
            duplicate_of?: number | null;
            fav_count?: string | null;
            is_banned?: boolean | null;
            is_deleted?: boolean | null;
            is_flagged?: boolean | null;
            is_pending?: boolean | null;
            observation_id?: string | null;
            observed_at_us?: string | null;
            /** Format: int64 */
            ordinal: number;
            parent_id?: string | null;
            post_id?: string | null;
            rating?: string | null;
            rating_conflict: boolean;
            /** Format: int32 */
            record_count: number;
            record_id?: string | null;
            score?: string | null;
            source_id: string;
            source_issues?: string | null;
            source_priority?: string | null;
            stored_bytes: string;
            stored_extension: string;
            /** Format: int32 */
            stored_height?: number | null;
            /** Format: int32 */
            stored_width?: number | null;
            tags_known: boolean;
            time_quality: string;
            up_score?: string | null;
            updated_at_us?: string | null;
        };
        /** @enum {string} */
        RankingMode: "rank" | "select";
        /** @enum {string} */
        RankingOrder: "main" | "rescue" | "input";
        RankingPage: {
            artifact_id: string;
            /** Format: int64 */
            count?: number | null;
            items: components["schemas"]["RankingRow"][];
            next_cursor?: string | null;
        };
        RankingPageRequest: {
            cursor?: string | null;
            filter?: components["schemas"]["RankingFilter"];
            limit?: number | null;
        };
        RankingParameters: {
            artist_enabled: boolean;
            /** Format: double */
            artist_weight: number;
            /** Format: int32 */
            cohort_minimum: number;
            damage_enabled: boolean;
            /** Format: double */
            damage_weight: number;
            exclude_banned: boolean;
            /** Format: int32 */
            minimum_stored_side?: number | null;
            mode: components["schemas"]["RankingMode"];
            quotas: number[];
            ratings: string[];
            seed: string;
            time_enabled: boolean;
            /** Format: double */
            time_weight: number;
            /** Format: double */
            vote_weight: number;
            votes_enabled: boolean;
        };
        RankingRatingSummary: {
            /** Format: int64 */
            artist_used: number;
            cohort_counts: number[];
            /** Format: int64 */
            eligible: number;
            /** Format: int64 */
            favorite_baseline_overlap: number;
            /** Format: double */
            q0: number;
            quotas: number[];
            rating: string;
            selected: number[];
            /** Format: int64 */
            time_fallback: number;
            /** Format: int64 */
            time_used: number;
            /** Format: int64 */
            valid_heat: number;
        };
        /** @enum {string} */
        RankingRoute: "ineligible" | "ranked" | "main" | "rescue" | "audit" | "budget_rejected";
        RankingRow: {
            input: components["schemas"]["RankingInput"];
            scores: components["schemas"]["RankingScores"];
        };
        RankingScores: {
            /** Format: double */
            a?: number | null;
            /** Format: int64 */
            artist_support: number;
            /** Format: double */
            c?: number | null;
            /** Format: int32 */
            cohort_level?: number | null;
            /** Format: int64 */
            duplicate_of?: number | null;
            eligibility: components["schemas"]["RankingEligibility"];
            /** Format: double */
            g?: number | null;
            /** Format: int64 */
            local_count: number;
            /** Format: double */
            local_percentile?: number | null;
            /** Format: int64 */
            main_rank?: number | null;
            /** Format: double */
            main_score?: number | null;
            missing_flags: string[];
            /** Format: int64 */
            ordinal: number;
            rating?: string | null;
            /** Format: int64 */
            rescue_rank?: number | null;
            /** Format: double */
            rescue_score?: number | null;
            selected_route: components["schemas"]["RankingRoute"];
            /** Format: double */
            support_k?: number | null;
            /** Format: double */
            t?: number | null;
            time_reason: string;
            /** Format: double */
            v?: number | null;
        };
        RankingSummary: {
            created_at: string;
            eligibility_counts: {
                [key: string]: number;
            };
            /** Format: int64 */
            eligible_count: number;
            /** Format: int64 */
            input_count: number;
            input_sha256: string;
            missing_counts: {
                [key: string]: number;
            };
            parameters: components["schemas"]["RankingParameters"];
            ratings: components["schemas"]["RankingRatingSummary"][];
            /** Format: int32 */
            schema_version: number;
        };
        RankingWorksetRequest: {
            filter?: components["schemas"]["RankingFilter"];
            idempotency_key: string;
            name: string;
        };
        RatingBases: {
            builds: components["schemas"]["RatingBuild"][];
            items: components["schemas"]["RatingBasis"][];
        };
        RatingBasis: {
            active: boolean;
            bytes: string;
            fixed: boolean;
            generation: string;
            incremental: boolean;
            last_used_millis: string;
            rating: string;
            /** Format: int64 */
            records: number;
            /** Format: int64 */
            sequence: number;
            source_id: string;
        };
        RatingBuild: {
            completed: string[];
            current_rating?: string | null;
            error?: string | null;
            source_id: string;
            state: string;
        };
        RawMetadata: {
            bytes?: string | null;
            format?: string | null;
            json?: string | null;
            observation_id: string;
            schema_id?: string | null;
            status: string;
            version: components["schemas"]["ReadVersion"];
        };
        RawMetadataQuery: {
            version: string;
        };
        ReadProcessMemory: {
            peak_resident_bytes: string;
            private_bytes: string;
            resident_bytes: string;
        };
        ReadServiceStatus: {
            cache: components["schemas"]["PreviewCacheStatus"];
            previews: components["schemas"]["PreviewActivity"];
            process_memory?: null | components["schemas"]["ReadProcessMemory"];
            /** Format: int32 */
            protocol_version: number;
            query_cache: components["schemas"]["QueryCacheStatus"];
            query_limits: components["schemas"]["QueryResourceLimits"];
            resources: components["schemas"]["ResourceStatus"][];
        };
        ReadVersion: {
            analysis_sequence: string;
            catalog_sequence: string;
            consistency: string;
            generation: string;
            library_id: string;
            token: string;
        };
        RelinkSource: {
            index_root: string;
            media_root: string;
        };
        ResourceRequirements: {
            /** Format: int32 */
            cpu_slots: number;
            gpu: boolean;
            media_reads: boolean;
            /** Format: int64 */
            memory_bytes: number;
        };
        ResourceStatus: {
            active: number;
            byte_budget: string;
            /** Format: int64 */
            cancelled_waiting: number;
            class: string;
            /** Format: int64 */
            completed: number;
            concurrency: number;
            /** Format: int64 */
            max_wait_ms: number;
            peak_reserved_bytes: string;
            queue_limit: number;
            queued: number;
            /** Format: int64 */
            rejected: number;
            reserved_bytes: string;
            /** Format: int64 */
            started: number;
            /** Format: int64 */
            wait_ms: number;
            /** Format: int64 */
            work_ms: number;
        };
        ResultAssets: {
            /** Format: int64 */
            count: number;
            page: components["schemas"]["AssetPage"];
            result_id: string;
        };
        /** @enum {string} */
        ResultState: "queued" | "running" | "ready" | "cancelled" | "failed" | "interrupted" | "released";
        ResultValidity: {
            current: boolean;
            issue?: string | null;
            result_id: string;
        };
        RunQuery: {
            spec: components["schemas"]["QuerySpec"];
        };
        SaveDraft: {
            /** Format: int64 */
            expected_revision: number;
            /** Format: int32 */
            schema_version: number;
            value: unknown;
        };
        SaveQuery: {
            /** Format: int64 */
            expected_revision?: number | null;
            name: string;
            spec: components["schemas"]["QuerySpec"];
        };
        ScalarInput: {
            /** @enum {string} */
            kind: "stored_bytes";
        } | {
            /** @enum {string} */
            kind: "origin_width";
        } | {
            artifact_id: string;
            /** @enum {string} */
            kind: "artifact";
        };
        ScalarValue: {
            /** @enum {string} */
            status: "available";
            value: string;
        } | {
            reason: string;
            /** @enum {string} */
            status: "missing";
        } | {
            code: string;
            message: string;
            /** @enum {string} */
            status: "failed";
        } | {
            reason: string;
            /** @enum {string} */
            status: "uncomputed";
        };
        /** @enum {string} */
        ScopeOperation: "replace" | "add" | "remove" | "intersect";
        ScopeRef: {
            project_id: string;
            target: components["schemas"]["ScopeTarget"];
        };
        ScopeTarget: {
            /** @enum {string} */
            kind: "source";
            revision: string;
            source_id: string;
        } | {
            collection_id: string;
            /** @enum {string} */
            kind: "workset";
        } | {
            /** @enum {string} */
            kind: "query_result";
            result_id: string;
        } | {
            /** @enum {string} */
            kind: "selection";
            /** Format: int64 */
            revision: number;
        };
        Selection: {
            base_result?: string | null;
            /** Format: int64 */
            count: number;
            /** Format: int64 */
            excluded_count: number;
            /** Format: int64 */
            revision: number;
        };
        SetCacheQuota: {
            /** Format: int32 */
            quota_mib: number;
        };
        SetQueryCache: {
            /** Format: int32 */
            max_age_days: number;
            /** Format: int32 */
            quota_mib: number;
        };
        SetQueryMemory: {
            /** Format: int32 */
            memory_gib: number;
        };
        SetRatingRetention: {
            fixed: boolean;
        };
        SetResultRetention: {
            fixed: boolean;
            tier: string;
        };
        SettingsStatus: {
            cache: components["schemas"]["CacheSettings"];
            query_limits: components["schemas"]["QueryResourceLimits"];
            storage: components["schemas"]["CacheStorageOverview"];
        };
        Source: {
            available: boolean;
            /** Format: int64 */
            count?: number | null;
            enumeration: string;
            id: string;
            issue?: string | null;
            kind: string;
            name: string;
            revision?: string | null;
        };
        SourceRelinked: {
            impact: string;
            revision: string;
            source_id: string;
        };
        Sources: {
            items: components["schemas"]["Source"][];
        };
        SubmitJob: {
            /** Format: int64 */
            delay_ms?: number;
            idempotency_key: string;
            scope?: null | components["schemas"]["ScopeRef"];
            /** Format: int64 */
            selection_revision?: number | null;
        };
        ToolSubmission: {
            /** Format: int64 */
            delay_ms?: number;
            idempotency_key: string;
            run: components["schemas"]["OperatorRun"];
            scope: components["schemas"]["ScopeRef"];
        };
    };
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    list_rating_bases: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RatingBases"];
                };
            };
        };
    };
    cancel_rating_basis_build: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                source_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
    set_rating_basis_retention: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                source_id: string;
                rating: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SetRatingRetention"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
    release_rating_basis: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                source_id: string;
                rating: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
    health: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Health"];
                };
            };
        };
    };
    operators: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Operators"];
                };
            };
        };
    };
    preference: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                key: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MaybePreference"];
                };
            };
        };
    };
    save_preference: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                key: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SaveDraft"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Preference"];
                };
            };
        };
    };
    projects: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Projects"];
                };
            };
        };
    };
    create_project: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateProject"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Project"];
                };
            };
            400: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    open_project: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["OpenProject"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Project"];
                };
            };
        };
    };
    project: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Project"];
                };
            };
        };
    };
    artifacts: {
        parameters: {
            query?: {
                cursor?: string;
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Artifacts"];
                };
            };
        };
    };
    get_artifact: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                artifact_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Artifact"];
                };
            };
        };
    };
    ranking_summary: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                artifact_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RankingSummary"];
                };
            };
        };
    };
    ranking_evidence: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                artifact_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RankingEvidence"];
                };
            };
        };
    };
    ranking_rows: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                artifact_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RankingPageRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RankingPage"];
                };
            };
        };
    };
    ranking_workset: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                artifact_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RankingWorksetRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Collection"];
                };
            };
        };
    };
    release_artifact: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                artifact_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Artifact"];
                };
            };
        };
    };
    rows: {
        parameters: {
            query?: {
                cursor?: string;
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
                artifact_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ArtifactPage"];
                };
            };
        };
    };
    verify: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                artifact_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Artifact"];
                };
            };
        };
    };
    assets: {
        parameters: {
            query?: {
                source_id?: string;
                collection_id?: string;
                selection?: boolean;
                cursor?: string;
                order?: components["schemas"]["QueryOrder"];
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AssetPage"];
                };
            };
        };
    };
    list_cache_entries: {
        parameters: {
            query?: {
                cursor?: string;
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CacheEntries"];
                };
            };
        };
    };
    keep_cache_session: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
    close_project: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ProjectClose"];
                };
            };
        };
    };
    collections: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Collections"];
                };
            };
        };
    };
    create_collection: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CreateCollection"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Collection"];
                };
            };
        };
    };
    draft: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                module_id: string;
                instance_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MaybeDraft"];
                };
            };
        };
    };
    save_draft: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                module_id: string;
                instance_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SaveDraft"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Draft"];
                };
            };
        };
    };
    events: {
        parameters: {
            query?: {
                after?: number;
            };
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "text/event-stream": components["schemas"]["ProjectEvent"];
                };
            };
        };
    };
    list_jobs: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Jobs"];
                };
            };
        };
    };
    submit_job: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SubmitJob"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Job"];
                };
            };
        };
    };
    artifact: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                job_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Published NDJSON manifest */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/x-ndjson": unknown;
                };
            };
        };
    };
    cancel_job: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                job_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Job"];
                };
            };
        };
    };
    ranking_job_result: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                job_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Artifact"];
                };
            };
        };
    };
    retry: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                job_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Job"];
                };
            };
        };
    };
    run: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                job_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["JobRun"];
                };
            };
        };
    };
    open_recent: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Project"];
                };
            };
        };
    };
    definitions: {
        parameters: {
            query?: {
                cursor?: string;
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryDefinitions"];
                };
            };
        };
    };
    create_definition: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SaveQuery"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryDefinition"];
                };
            };
        };
    };
    definition: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                query_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryDefinition"];
                };
            };
        };
    };
    update_definition: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                query_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SaveQuery"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryDefinition"];
                };
            };
        };
    };
    build: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                query_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["BuildQuery"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryResult"];
                };
            };
        };
    };
    results: {
        parameters: {
            query?: {
                cursor?: string;
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryResults"];
                };
            };
        };
    };
    run_query: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RunQuery"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryResult"];
                };
            };
        };
    };
    result: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                result_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryResult"];
                };
            };
        };
    };
    result_assets: {
        parameters: {
            query?: {
                cursor?: string;
                order?: components["schemas"]["QueryOrder"];
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
                result_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ResultAssets"];
                };
            };
        };
    };
    release_cache_entry: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                result_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryResult"];
                };
            };
        };
    };
    cancel: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                result_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryResult"];
                };
            };
        };
    };
    lease_result: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                result_id: string;
                lease_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
    release_result_lease: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                result_id: string;
                lease_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
    release: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                result_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryResult"];
                };
            };
        };
    };
    set_result_retention: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                result_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SetResultRetention"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryResult"];
                };
            };
        };
    };
    validity: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                result_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ResultValidity"];
                };
            };
        };
    };
    cancel_read_subscription: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                request_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
    capture: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CaptureScope"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryResult"];
                };
            };
        };
    };
    selection: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Selection"];
                };
            };
        };
    };
    change_selection: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ChangeSelection"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Selection"];
                };
            };
        };
    };
    select_scope: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ChangeSelectionScope"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Selection"];
                };
            };
        };
    };
    sources: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Sources"];
                };
            };
        };
    };
    attach_source: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AttachSource"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Source"];
                };
            };
        };
    };
    asset_detail: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                source_id: string;
                asset_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Asset"];
                };
            };
        };
    };
    media: {
        parameters: {
            query?: {
                edge?: number;
                request_id?: string;
                /** @description interactive, background or prefetch */
                priority?: string;
                /** @description Cold generation input byte limit, at most 64 MiB */
                max_source_bytes?: number;
            };
            header?: never;
            path: {
                project_id: string;
                source_id: string;
                asset_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Authenticated image bytes; x-studio-cache, x-studio-freshness and x-studio-verified-ms report cache verification */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "image/jpeg": unknown;
                };
            };
        };
    };
    metadata: {
        parameters: {
            query?: {
                cursor?: string;
                limit?: number;
                version?: string;
            };
            header?: never;
            path: {
                project_id: string;
                source_id: string;
                asset_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MetadataOverview"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    observations: {
        parameters: {
            query?: {
                cursor?: string;
                limit?: number;
                version?: string;
            };
            header?: never;
            path: {
                project_id: string;
                source_id: string;
                asset_id: string;
                record_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ObservationPage"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    raw_metadata: {
        parameters: {
            query: {
                version: string;
            };
            header?: never;
            path: {
                project_id: string;
                source_id: string;
                asset_id: string;
                record_id: string;
                observation_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RawMetadata"];
                };
            };
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ApiError"];
                };
            };
        };
    };
    fields: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                source_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["FieldDirectory"];
                };
            };
        };
    };
    prebuild_rating_bases: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                source_id: string;
            };
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RatingBuild"];
                };
            };
        };
    };
    relink: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                source_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RelinkSource"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SourceRelinked"];
                };
            };
        };
    };
    submit: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ToolSubmission"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["Job"];
                };
            };
        };
    };
    validate_scope: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CaptureScope"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
    status: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ReadServiceStatus"];
                };
            };
        };
    };
    configure: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SetCacheQuota"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PreviewCacheStatus"];
                };
            };
        };
    };
    clear: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["PreviewCacheStatus"];
                };
            };
        };
    };
    configure_query: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SetQueryMemory"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryResourceLimits"];
                };
            };
        };
    };
    configure_query_cache: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SetQueryCache"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryCacheStatus"];
                };
            };
        };
    };
    clear_query_cache: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["QueryCacheStatus"];
                };
            };
        };
    };
    read_settings: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SettingsStatus"];
                };
            };
        };
    };
    configure_cache_settings: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["CacheSettings"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SettingsStatus"];
                };
            };
        };
    };
    clear_cache_tier: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ClearCacheTier"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SettingsStatus"];
                };
            };
        };
    };
    shutdown: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
}
