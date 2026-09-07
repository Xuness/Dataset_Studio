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
