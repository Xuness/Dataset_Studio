export interface paths {
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
        Asset: {
            bytes: string;
            extension: string;
            key: components["schemas"]["AssetKey"];
            name: string;
            selected: boolean;
            source_name: string;
        };
        AssetKey: {
            asset_id: string;
            source_id: string;
        };
        AssetPage: {
            items: components["schemas"]["Asset"][];
            next_cursor?: string | null;
            revision: string;
        };
        AssetRecord: {
            origin_observation_id?: string | null;
            post_id?: string | null;
            record_id: string;
            source_md5?: string | null;
            storage_profile?: string | null;
        };
        AttachSource: {
            index_root?: string | null;
            kind: string;
            media_root?: string | null;
            name: string;
        };
        ChangeSelection: {
            add: components["schemas"]["AssetKey"][];
            clear: boolean;
            /** Format: int64 */
            expected_revision: number;
            remove: components["schemas"]["AssetKey"][];
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
        };
        CreateProject: {
            name: string;
            parent_directory?: string | null;
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
            operator: string;
            project_id: string;
            status: string;
            /** Format: int64 */
            total: number;
        };
        Jobs: {
            items: components["schemas"]["Job"][];
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
        OkResponse: {
            ok: boolean;
        };
        OpenProject: {
            directory: string;
        };
        Project: {
            created_at: string;
            directory: string;
            id: string;
            name: string;
            /** Format: int64 */
            revision: number;
        };
        ProjectEvent: {
            kind: string;
            project_id: string;
            resource_id: string;
            /** Format: int64 */
            sequence: number;
        };
        Projects: {
            items: components["schemas"]["Project"][];
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
        ReadVersion: {
            analysis_sequence: string;
            catalog_sequence: string;
            consistency: string;
            generation: string;
            library_id: string;
            token: string;
        };
        Selection: {
            /** Format: int64 */
            count: number;
            /** Format: int64 */
            revision: number;
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
        Sources: {
            items: components["schemas"]["Source"][];
        };
        SubmitJob: {
            /** Format: int64 */
            delay_ms?: number;
            idempotency_key: string;
            /** Format: int64 */
            selection_revision: number;
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
    assets: {
        parameters: {
            query?: {
                source_id?: string;
                collection_id?: string;
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
                    "application/json": components["schemas"]["AssetPage"];
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
    media: {
        parameters: {
            query?: {
                edge?: number;
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
            /** @description Authenticated image bytes */
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
