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
        post?: never;
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
        BuildQuery: {
            /** Format: int64 */
            expected_revision: number;
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
        /** @enum {string} */
        ObservationRule: "current_post" | "any_observation";
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
        QueryOperator: "eq" | "ne" | "gte" | "lte" | "has_tag" | "is_missing" | "is_present";
        /** @enum {string} */
        QueryOrder: "asset_key_asc" | "asset_key_desc";
        QueryResult: {
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
        RelinkSource: {
            index_root: string;
            media_root: string;
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
        SaveQuery: {
            /** Format: int64 */
            expected_revision?: number | null;
            name: string;
            spec: components["schemas"]["QuerySpec"];
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
                selection?: boolean;
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
