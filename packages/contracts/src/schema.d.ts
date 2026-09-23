export interface paths {
    "/v1/cache/projects": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["cache_projects"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/cache/projects/{project_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["project_cache_inventory"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/cache/projects/{project_id}/members/{result_id}/release": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["release_project_cache_member"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/cache/projects/{project_id}/members/{result_id}/retention": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put: operations["retain_project_cache_member"];
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/cache/projects/{project_id}/ranked/{key}/release": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["release_project_ranked_index"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
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
    "/v1/llm/generate": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["llm_generate"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/invocations/{id}/cancel": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["llm_cancel"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/models": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["llm_save_model"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/models/{id}/parameters": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["llm_model_parameters"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/models/{id}/remove": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["llm_remove_model"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/parameters": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["llm_parameters"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/prepare": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["llm_prepare"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/presets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["llm_presets"];
        put?: never;
        post: operations["llm_save_preset"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/presets/{id}/remove": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["llm_remove_preset"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/providers": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["llm_providers"];
        put?: never;
        post: operations["llm_save_provider"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/providers/{id}/catalog": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["llm_catalog"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/providers/{id}/catalog/refresh": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["llm_refresh_catalog"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/providers/{id}/models": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["llm_models"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/providers/{id}/remove": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["llm_remove_provider"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/stream": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["llm_stream"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/system-prompts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["llm_system_prompts"];
        put?: never;
        post: operations["llm_save_system_prompt"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/system-prompts/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["llm_system_prompt"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/llm/system-prompts/{id}/remove": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["llm_remove_system_prompt"];
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
    "/v1/projects/{project_id}/aesthetic/analysis/experiments": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_experiments"];
        put?: never;
        post: operations["aesthetic_experiment_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/experiments/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_experiment"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/experiments/{id}/run": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["aesthetic_experiment_run"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/jobs": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_analysis_jobs"];
        put?: never;
        post: operations["aesthetic_analysis_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/jobs/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_analysis_job"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/jobs/{id}/comparison": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_comparison_rows"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/jobs/{id}/control": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["aesthetic_analysis_control"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/reviews": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["aesthetic_review_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/snapshots/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_snapshot"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/snapshots/{id}/candidates/{ordinal}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_ranking_candidate"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/snapshots/{id}/reviews": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_reviews"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/snapshots/{id}/rows": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_ranking_rows"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/analysis/snapshots/{id}/select": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["aesthetic_ranking_select"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/backup": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["aesthetic_backup"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/capabilities": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_capabilities"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/creation-intents/{id}/abandon": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["aesthetic_abandon_creation"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/metrics": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_metrics"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/preflight": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["aesthetic_preflight"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/stages": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_stages"];
        put?: never;
        post: operations["aesthetic_create"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/stages/{id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_stage"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/stages/{id}/batches": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_batches"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/stages/{id}/batches/{batch}/attempts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_attempts"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/stages/{id}/batches/{batch}/retry": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["aesthetic_retry"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/stages/{id}/candidates": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["aesthetic_candidates"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/stages/{id}/candidates/{ordinal}/disposition": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["aesthetic_decide_candidate"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/aesthetic/stages/{id}/control": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["aesthetic_control"];
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
    "/v1/projects/{project_id}/artifacts/{artifact_id}/ranking/count": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["ranking_count"];
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
    "/v1/projects/{project_id}/artifacts/{artifact_id}/ranking/rows/{ordinal}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["ranking_row"];
        put?: never;
        post?: never;
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
    "/v1/projects/{project_id}/assets/summaries": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["asset_summaries"];
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
    "/v1/projects/{project_id}/job-history": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["jobs"];
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
    "/v1/projects/{project_id}/member-writes/{operation_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["member_write_progress"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/member-writes/{operation_id}/cancel": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["cancel_member_write"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/objects/{kind}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["list"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/objects/{kind}/{object_id}": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["read"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch: operations["edit"];
        trace?: never;
    };
    "/v1/projects/{project_id}/objects/{kind}/{object_id}/actions": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["action"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/objects/{kind}/{object_id}/links": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["links"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/objects/{kind}/{object_id}/reveal": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["reveal"];
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
    "/v1/projects/{project_id}/presets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["presets"];
        put?: never;
        post: operations["save_preset"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/presets/{preset_id}/delete": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["delete_preset"];
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
    "/v1/projects/{project_id}/ranking-browse": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["ranking_browse_info"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/ranking-browse/assets": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["ranking_browse_assets"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/ranking-browse/lease": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["ranking_scope_lease"];
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
    "/v1/projects/{project_id}/selection/history": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["history"];
        put?: never;
        post: operations["restore"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/projects/{project_id}/selection/members": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        post: operations["selection_members"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
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
    "/v1/settings/editing": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: operations["editing"];
        put: operations["configure_editing"];
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
        AestheticAnalysisControl: {
            action: string;
        };
        AestheticAnalysisCreate: {
            idempotency_key: string;
            name: string;
            spec: components["schemas"]["AestheticAnalysisSpec"];
        };
        AestheticAnalysisInput: {
            /** Format: int64 */
            candidates: number;
            /** Format: int64 */
            evidence_watermark: number;
            /** Format: int64 */
            observations: number;
            /** Format: int64 */
            review_watermark: number;
            stage_config_hash: string;
            stage_id: string;
        };
        AestheticAnalysisJob: {
            created_at: string;
            error?: string | null;
            id: string;
            input: components["schemas"]["AestheticAnalysisInput"];
            phase: string;
            /** Format: int64 */
            progress: number;
            request: components["schemas"]["AestheticAnalysisCreate"];
            result?: null | components["schemas"]["AestheticAnalysisSummary"];
            state: string;
            /** Format: int64 */
            total: number;
        };
        AestheticAnalysisJobs: {
            items: components["schemas"]["AestheticAnalysisJob"][];
            next_cursor?: string | null;
        };
        AestheticAnalysisSpec: {
            config: components["schemas"]["AestheticFit"];
            experiment_id?: string | null;
            /** @enum {string} */
            kind: "fit";
            variant?: string | null;
        } | {
            /** @enum {string} */
            kind: "compare";
            left: string;
            right: string;
        } | {
            filter: components["schemas"]["AestheticRankingFilter"];
            /** @enum {string} */
            kind: "derive";
            /** Format: int64 */
            review_watermark?: number | null;
            snapshot_id: string;
        };
        AestheticAnalysisSummary: (components["schemas"]["AestheticFitSummary"] & {
            /** @enum {string} */
            kind: "fit";
        }) | {
            groups: components["schemas"]["AestheticComparisonGroup"][];
            /** @enum {string} */
            kind: "compare";
        } | {
            collection_id: string;
            /** Format: int64 */
            count: number;
            /** @enum {string} */
            kind: "derive";
        };
        AestheticAttempt: {
            /** Format: int64 */
            batch: number;
            created_at: string;
            failure?: null | components["schemas"]["LlmFailure"];
            id: string;
            receipt?: null | components["schemas"]["AestheticReceipt"];
            semantic_request_hash?: string | null;
            state: string;
        };
        AestheticAttempts: {
            items: components["schemas"]["AestheticAttempt"][];
        };
        AestheticBackup: {
            relative_path: string;
        };
        AestheticBatch: {
            attempt_id?: string | null;
            error?: string | null;
            members: components["schemas"]["AestheticMember"][];
            observation?: null | components["schemas"]["AestheticObservation"];
            rating: string;
            /** Format: int64 */
            sequence: number;
            stage_id: string;
            state: string;
        };
        AestheticBatches: {
            items: components["schemas"]["AestheticBatch"][];
            next_cursor?: string | null;
        };
        AestheticCandidate: {
            basis: string;
            /** Format: int64 */
            bytes: number;
            content_version: string;
            disposition: components["schemas"]["AestheticDisposition"];
            disposition_reason?: string | null;
            /** Format: int32 */
            exposures: number;
            key: components["schemas"]["AssetKey"];
            /** Format: int64 */
            ordinal: number;
            protected: boolean;
            rating: string;
            /** Format: int32 */
            year?: number | null;
        };
        AestheticCandidateDecision: {
            action: components["schemas"]["AestheticDispositionAction"];
            idempotency_key: string;
            reason: string;
        };
        AestheticCandidates: {
            items: components["schemas"]["AestheticCandidate"][];
            next_cursor?: string | null;
        };
        AestheticCapabilities: {
            /** Format: int32 */
            batch_size: number;
            /** Format: int64 */
            max_image_bytes: number;
            /** Format: int64 */
            max_request_bytes: number;
            /** Format: int64 */
            max_stage_candidates: number;
            /** Format: int32 */
            version: number;
        };
        AestheticComparisonGroup: {
            comparable: boolean;
            /** Format: int64 */
            elite_disagreements: number;
            /** Format: int64 */
            matched: number;
            /** Format: double */
            mean_absolute_percentile_delta?: number | null;
            /** Format: double */
            middle_mean_absolute_percentile_delta?: number | null;
            /** Format: double */
            rank_correlation?: number | null;
            rating: string;
            reason?: string | null;
            /** Format: double */
            top20_jaccard?: number | null;
        };
        AestheticComparisonRow: {
            comparable: boolean;
            key: components["schemas"]["AssetKey"];
            /** Format: double */
            left_percentile?: number | null;
            left_protected: boolean;
            /** Format: double */
            percentile_delta?: number | null;
            /** Format: int64 */
            position: number;
            rating: string;
            reason?: string | null;
            /** Format: double */
            right_percentile?: number | null;
            right_protected?: boolean | null;
        };
        AestheticComparisonRows: {
            items: components["schemas"]["AestheticComparisonRow"][];
            next_cursor?: string | null;
        };
        AestheticConfig: {
            execution?: null | components["schemas"]["AestheticExecution"];
            grouping_policy: string;
            image_policy: string;
            /** Format: int64 */
            max_image_bytes: number;
            /** Format: int64 */
            max_request_bytes: number;
            /** @description Credential-free, resolved configuration; contains no image bytes. */
            model: components["schemas"]["LlmInvocationSnapshot"];
            observation_policy: string;
            request: components["schemas"]["AestheticCreate"];
            sources: components["schemas"]["QuerySourceVersion"][];
            /** Format: int32 */
            version: number;
        };
        AestheticControl: {
            action: string;
        };
        AestheticCreate: {
            collection_id: string;
            /** Format: int32 */
            concurrency: number;
            expected_input_version?: string | null;
            /** Format: int32 */
            exposures: number;
            idempotency_key: string;
            /** Format: int32 */
            max_calls: number;
            model_id: string;
            name: string;
            overrides?: {
                [key: string]: unknown;
            };
            system_prompt_id: string;
        };
        /** @enum {string} */
        AestheticDisposition: "active" | "needs_review" | "rejudge" | "excluded";
        /** @enum {string} */
        AestheticDispositionAction: "rejudge" | "exclude";
        AestheticEstimator: {
            /** Format: int32 */
            iterations: number;
            /** @description davidson_v1 (batch-normalized composite objective) or borda_v1 (baseline). */
            kind: string;
            /** Format: double */
            regularization: number;
            /** Format: double */
            tie_strength: number;
        };
        AestheticExecution: {
            /** Format: int32 */
            business_schema_version: number;
            encoder_version: string;
            input_version: string;
            sampler_version: string;
            template_version: string;
        };
        AestheticExperiment: {
            created_at: string;
            id: string;
            inputs: components["schemas"]["AestheticAnalysisInput"][];
            request: components["schemas"]["AestheticExperimentCreate"];
        };
        AestheticExperimentCreate: {
            description: string;
            idempotency_key: string;
            name: string;
            variants: components["schemas"]["AestheticExperimentVariant"][];
        };
        AestheticExperimentVariant: {
            fit: components["schemas"]["AestheticFit"];
            label: string;
        };
        AestheticExperiments: {
            items: components["schemas"]["AestheticExperiment"][];
            next_cursor?: string | null;
        };
        AestheticFit: {
            estimator: components["schemas"]["AestheticEstimator"];
            /**
             * Format: int32
             * @description Refit two deterministic, disjoint sets of whole batches. Not a confidence interval.
             */
            stability_seed?: number | null;
            stage_id: string;
        };
        AestheticFitSummary: {
            converged: boolean;
            estimator_version: string;
            groups: components["schemas"]["AestheticRatingSummary"][];
            /** Format: int32 */
            iterations_completed: number;
            /** Format: double */
            max_update: number;
            stability_method: string;
            /** Format: int64 */
            working_bytes_estimate: number;
        };
        AestheticMember: {
            candidate: components["schemas"]["AestheticCandidate"];
            image_sha256?: string | null;
            label: string;
        };
        AestheticMetrics: {
            /** Format: int64 */
            active_requests: number;
            dispatch_health: string;
            /** Format: int64 */
            peak_request_bytes: number;
            /** Format: int64 */
            peak_write_bytes: number;
            /** Format: int64 */
            queued_write_bytes: number;
            /** Format: int64 */
            reserved_request_bytes: number;
            /** Format: int64 */
            retained_outcomes: number;
            storage_error_code?: string | null;
            /** Format: int64 */
            upload_budget_bytes_per_second: number;
            /** Format: int64 */
            uploaded_body_bytes: number;
        };
        AestheticObservation: {
            elite_candidates: string[];
            /** Format: int32 */
            schema_version: number;
            tiers: string[][];
            unjudgeable: components["schemas"]["AestheticUnjudgeable"][];
        };
        AestheticPreflight: {
            admitted: boolean;
            capabilities: components["schemas"]["AestheticCapabilities"];
            input_version: string;
            rejection_code?: string | null;
            rejection_reason?: string | null;
            /** Format: int64 */
            total: number;
        };
        AestheticRankingFilter: {
            /** Format: int64 */
            component?: number | null;
            /**
             * @description Union protected candidates into the selection after quality filters;
             *     Rating and year restrictions still apply.
             */
            include_protected?: boolean;
            /** Format: int32 */
            max_exposures?: number | null;
            /** Format: double */
            min_split_delta?: number | null;
            needs_review?: boolean;
            protected_only?: boolean;
            /** Format: int64 */
            rank_from?: number | null;
            /** Format: int64 */
            rank_to?: number | null;
            ratings?: string[];
            /** Format: double */
            top_percent?: number | null;
            /** Format: int32 */
            year_from?: number | null;
            /** Format: int32 */
            year_to?: number | null;
        };
        AestheticRankingQuery: {
            after?: string | null;
            filter: components["schemas"]["AestheticRankingFilter"];
            /** Format: int32 */
            limit?: number | null;
        };
        AestheticRankingRow: {
            /** Format: int64 */
            component?: number | null;
            /** Format: double */
            component_percentile?: number | null;
            /** Format: int64 */
            component_size: number;
            content_version: string;
            /** Format: int32 */
            cross_year_exposures: number;
            /** Format: double */
            disagreement?: number | null;
            /** Format: int32 */
            exposures: number;
            key: components["schemas"]["AssetKey"];
            needs_review: boolean;
            /** Format: int32 */
            opponent_bins: number;
            /** Format: double */
            opponent_diversity_estimate?: number | null;
            /** Format: int64 */
            ordinal: number;
            /** Format: double */
            percentile?: number | null;
            /**
             * Format: int64
             * @description Pagination position only, never a cross-Rating/global aesthetic rank.
             */
            position: number;
            protected: boolean;
            /** Format: int64 */
            rank_max?: number | null;
            /** Format: int64 */
            rank_min?: number | null;
            rating: string;
            /** Format: int64 */
            rating_rank_max?: number | null;
            /** Format: int64 */
            rating_rank_min?: number | null;
            /** Format: double */
            score?: number | null;
            /** Format: double */
            split_percentile_delta?: number | null;
            /** Format: int32 */
            unjudgeable: number;
            /** Format: int32 */
            year?: number | null;
        };
        AestheticRankingRows: {
            items: components["schemas"]["AestheticRankingRow"][];
            next_cursor?: string | null;
        };
        AestheticRankingSelection: {
            items: components["schemas"]["AestheticSelectionRow"][];
            next_cursor?: string | null;
            /** Format: int64 */
            review_watermark: number;
            /** Format: int64 */
            scanned: number;
        };
        AestheticRatingSummary: {
            /** Format: int64 */
            candidates: number;
            /** Format: int64 */
            compared: number;
            /** Format: int64 */
            components: number;
            /** Format: int64 */
            cross_year_batches: number;
            fully_connected: boolean;
            /** Format: int64 */
            judged: number;
            /** Format: int64 */
            protected: number;
            rating: string;
            /** Format: int64 */
            split_comparable: number;
        };
        AestheticReceipt: {
            model?: string | null;
            outputs: components["schemas"]["LlmOutput"][];
            provider_request_id?: string | null;
            response_id?: string | null;
            usage: components["schemas"]["LlmUsage"];
        };
        AestheticRetry: {
            acknowledge_possible_charge: boolean;
        };
        AestheticReview: {
            created_at: string;
            request: components["schemas"]["AestheticReviewCreate"];
            /** Format: int64 */
            sequence: number;
        };
        AestheticReviewCreate: {
            /** @description protect, confirm_elite, release, defer. Decisions never change statistical scores. */
            decision: string;
            idempotency_key: string;
            /** Format: int64 */
            ordinal: number;
            reason: string;
            reviewer: string;
            snapshot_id: string;
        };
        AestheticReviews: {
            items: components["schemas"]["AestheticReview"][];
            next_cursor?: string | null;
        };
        AestheticSelectionRow: {
            effective_protected: boolean;
            ranking: components["schemas"]["AestheticRankingRow"];
        };
        AestheticStage: {
            /** Format: int64 */
            accepted: number;
            /** Format: int64 */
            attempts: number;
            /** Format: int64 */
            comparable: number;
            config: components["schemas"]["AestheticConfig"];
            config_hash: string;
            created_at: string;
            /** Format: int64 */
            eligible: number;
            error?: string | null;
            /** Format: int64 */
            excluded: number;
            /** Format: int64 */
            frozen: number;
            id: string;
            /** Format: int64 */
            input_tokens: number;
            /** Format: int64 */
            invalid: number;
            name: string;
            /** Format: int64 */
            output_tokens: number;
            /** Format: int64 */
            protected: number;
            state: string;
            /** Format: int64 */
            total: number;
            /** Format: int64 */
            unknown: number;
            /** Format: int64 */
            unresolved: number;
            /** Format: int64 */
            usage_unknown: number;
        };
        AestheticStages: {
            items: components["schemas"]["AestheticStage"][];
            next_cursor?: string | null;
        };
        AestheticUnjudgeable: {
            id: string;
            reason: string;
        };
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
            ranking?: null | components["schemas"]["AssetRanking"];
            selected: boolean;
            source_name: string;
            summary?: null | components["schemas"]["AssetSummary"];
        };
        AssetKey: {
            asset_id: string;
            source_id: string;
        };
        AssetKeysRequest: {
            keys: components["schemas"]["AssetKey"][];
        };
        AssetPage: {
            items: components["schemas"]["Asset"][];
            next_cursor?: string | null;
            preparing?: string | null;
            result_id?: string | null;
            revision: string;
            scan?: null | components["schemas"]["BrowseScan"];
            /** @description Reusable first-page cursor after a Danbooru ID or ranking position has been located. */
            start_cursor?: string | null;
        };
        AssetRanking: {
            artifact_id: string;
            eligibility: components["schemas"]["RankingEligibility"];
            /** Format: int64 */
            main_rank?: number | null;
            /** Format: double */
            main_score?: number | null;
            observation_id?: string | null;
            /** Format: int64 */
            ordinal: number;
            post_id?: string | null;
            rating?: string | null;
            record_id?: string | null;
            /** Format: int64 */
            rescue_rank?: number | null;
            /** Format: double */
            rescue_score?: number | null;
            v2?: null | components["schemas"]["RankingV2Scores"];
        };
        AssetRecord: {
            origin_observation_id?: string | null;
            post_id?: string | null;
            record_id: string;
            source_md5?: string | null;
            storage_profile?: string | null;
        };
        AssetSummaries: {
            items: components["schemas"]["AssetSummaryEntry"][];
            preparing: boolean;
        };
        AssetSummary: {
            issue?: string | null;
            post_count?: string | null;
            post_ids: string[];
            /** @description available, unlinked, preparing, unavailable, or unsupported. */
            status: string;
            version?: string | null;
        };
        AssetSummaryEntry: {
            key: components["schemas"]["AssetKey"];
            summary: components["schemas"]["AssetSummary"];
        };
        AttachSource: {
            index_root?: string | null;
            kind: string;
            media_root?: string | null;
            name: string;
        };
        BrowseScan: {
            /** Format: int64 */
            scanned: number;
            /** Format: int64 */
            total: number;
        };
        BuildQuery: {
            /** Format: int64 */
            expected_revision: number;
        };
        CacheCleanup: {
            error?: string | null;
            family_id: string;
            /** Format: int64 */
            processed: number;
            /** Format: int64 */
            removed: number;
            result_id: string;
            spec?: null | components["schemas"]["QuerySpec"];
            started_millis: string;
            state: string;
            /** Format: int64 */
            total: number;
            updated_millis: string;
        };
        CacheEntries: {
            cleanups: components["schemas"]["CacheCleanup"][];
            items: components["schemas"]["CacheEntry"][];
            next_cursor?: string | null;
        };
        CacheEntry: {
            estimated_bytes?: string | null;
            family_id: string;
            fixed: boolean;
            in_use: boolean;
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
        CacheMaintenance: {
            error?: string | null;
            phase: string;
            project_id?: string | null;
        };
        CacheMemberItem: {
            cached: boolean;
            can_release: boolean;
            estimated_bytes?: string | null;
            family_id: string;
            fixed: boolean;
            in_use: boolean;
            last_used_millis: string;
            /** Format: int64 */
            members: number;
            reason?: string | null;
            /** Format: int64 */
            reference_count: number;
            references: string[];
            result_id: string;
            session_only: boolean;
            spec: components["schemas"]["QuerySpec"];
            tier: string;
        };
        CacheProject: {
            directory: string;
            issue?: string | null;
            long_term_bytes: string;
            member_bytes: string;
            name: string;
            project_id: string;
            ranked_index_bytes: string;
            state: components["schemas"]["ProjectState"];
            temporary_bytes: string;
            total_bytes: string;
        };
        CacheProjects: {
            items: components["schemas"]["CacheProject"][];
        };
        CacheRankedItem: {
            bytes: string;
            in_use: boolean;
            key: string;
            label: string;
            last_used_millis: string;
            /** Format: int64 */
            members: number;
            path: string;
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
            ranked_index_bytes: string;
            rating_basis_bytes: string;
            reusable_bytes: string;
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
        ConfigureEditing: {
            /** Format: int64 */
            expected_revision: number;
            /** Format: int32 */
            undo_limit: number;
        };
        CreateCollection: {
            name: string;
            scope?: null | components["schemas"]["ScopeRef"];
        };
        CreateProject: {
            name: string;
            parent_directory?: string | null;
        };
        DeleteToolPreset: {
            /** Format: int64 */
            expected_revision: number;
        };
        DiscoverLlmModels: {
            /** Format: int64 */
            expected_revision: number;
            invocation_id: string;
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
        /** @enum {string} */
        DuplicateHeat: "highest" | "sum";
        EditObject: {
            /** Format: int64 */
            expected_revision: number;
            name: string;
            notes: string;
        };
        EditingSettings: {
            /** Format: int64 */
            revision: number;
            /** Format: int32 */
            undo_limit: number;
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
        EraPreference: {
            /** Format: double */
            bonus: number;
            /** Format: int32 */
            from_year: number;
            /**
             * Format: int32
             * @description Thousandths of the final per-rating retained count. Null means soft preference only.
             */
            target_share?: number | null;
            /** Format: int32 */
            through_year: number;
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
        HistoryAction: {
            action: components["schemas"]["HistoryActionKind"];
            /** Format: int64 */
            expected_revision: number;
        };
        /** @enum {string} */
        HistoryActionKind: "undo" | "redo" | "clear";
        HistoryStatus: {
            /** Format: int32 */
            limit: number;
            redo_label?: string | null;
            /** Format: int32 */
            redo_steps: number;
            selection: components["schemas"]["Selection"];
            undo_label?: string | null;
            /** Format: int32 */
            undo_steps: number;
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
        JobPhaseTiming: {
            /** Format: int64 */
            elapsed_ms: number;
            name: string;
            rating?: string | null;
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
            rating?: string | null;
            telemetry?: null | components["schemas"]["JobTelemetry"];
            /** Format: int64 */
            total: number;
        };
        JobTelemetry: {
            finished_at?: string | null;
            heartbeat_at: string;
            phases: components["schemas"]["JobPhaseTiming"][];
            started_at: string;
            updated_at: string;
        };
        Jobs: {
            items: components["schemas"]["Job"][];
        };
        LlmCatalog: {
            fetched_at: string;
            models: components["schemas"]["LlmCatalogModel"][];
            provider_id: string;
            /** Format: int64 */
            provider_revision: number;
        };
        LlmCatalogModel: {
            capabilities: {
                [key: string]: components["schemas"]["LlmSupport"];
            };
            id: string;
            input_modalities: string[];
            /** Format: int64 */
            input_token_limit?: number | null;
            name: string;
            output_modalities: string[];
            /** Format: int64 */
            output_token_limit?: number | null;
        };
        LlmCatalogStatus: {
            catalog?: null | components["schemas"]["LlmCatalog"];
        };
        LlmConnectionConfig: {
            base_url: string;
            enabled: boolean;
            headers?: {
                [key: string]: string;
            };
            kind: components["schemas"]["LlmProviderKind"];
            name: string;
            network: components["schemas"]["LlmNetworkSettings"];
        };
        LlmContent: {
            text: string;
            /** @enum {string} */
            type: "text";
        } | {
            detail?: string | null;
            /** @enum {string} */
            type: "image";
            url: string;
        } | {
            arguments: unknown;
            id: string;
            name: string;
            signature?: string | null;
            /** @enum {string} */
            type: "tool_call";
        } | {
            id: string;
            name: string;
            /** @enum {string} */
            type: "tool_result";
            value: unknown;
        } | {
            text: string;
            /** @enum {string} */
            type: "reasoning";
        } | {
            text: string;
            /** @enum {string} */
            type: "refusal";
        };
        LlmEvent: {
            invocation_id: string;
            /** @enum {string} */
            type: "started";
        } | {
            /** Format: int32 */
            index: number;
            kind: string;
            text: string;
            tool_call_id?: string | null;
            /** @enum {string} */
            type: "delta";
        } | {
            response: components["schemas"]["LlmResponse"];
            /** @enum {string} */
            type: "completed";
        } | {
            error: components["schemas"]["LlmFailure"];
            /** @enum {string} */
            type: "failed";
        };
        LlmFailure: {
            code: string;
            /** Format: int32 */
            http_status?: number | null;
            message: string;
            outcome_unknown: boolean;
            provider_request_id?: string | null;
            retryable: boolean;
        };
        LlmInvocationRequest: {
            /** Format: int64 */
            expected_model_revision?: number | null;
            /** Format: int64 */
            expected_preset_revision?: number | null;
            /** Format: int64 */
            expected_provider_revision?: number | null;
            /** Format: int64 */
            expected_system_prompt_revision?: number | null;
            invocation_id: string;
            messages: components["schemas"]["LlmMessage"][];
            model_id: string;
            overrides?: {
                [key: string]: unknown;
            };
            preset_id?: string | null;
            /** @description Optional saved System Prompt. Cannot be combined with system/developer messages. */
            system_prompt_id?: string | null;
            tools?: components["schemas"]["LlmTool"][];
        };
        LlmInvocationSnapshot: {
            base_url: string;
            invocation_id: string;
            messages: components["schemas"]["LlmMessage"][];
            model_id: string;
            /** Format: int64 */
            model_revision: number;
            parameters?: {
                [key: string]: unknown;
            };
            preset_id?: string | null;
            /** Format: int64 */
            preset_revision?: number | null;
            protocol: components["schemas"]["LlmProtocol"];
            provider_id: string;
            provider_kind: components["schemas"]["LlmProviderKind"];
            /** Format: int64 */
            provider_revision: number;
            remote_model_id: string;
            /** Format: int32 */
            schema_version: number;
            system_prompt_id?: string | null;
            /** Format: int64 */
            system_prompt_revision?: number | null;
            tools?: components["schemas"]["LlmTool"][];
            warnings: string[];
        };
        LlmMessage: {
            content: components["schemas"]["LlmContent"][];
            role: components["schemas"]["LlmRole"];
        };
        LlmModel: {
            config: components["schemas"]["LlmModelConfig"];
            id: string;
            provider_id: string;
            /** Format: int64 */
            revision: number;
        };
        LlmModelConfig: {
            capability_overrides?: {
                [key: string]: components["schemas"]["LlmSupport"];
            };
            enabled: boolean;
            name: string;
            parameters?: {
                [key: string]: unknown;
            };
            protocol: components["schemas"]["LlmProtocol"];
            remote_model_id: string;
        };
        LlmModels: {
            items: components["schemas"]["LlmModel"][];
        };
        LlmNetworkSettings: {
            /** Format: int32 */
            connect_timeout_ms: number;
            /** Format: int32 */
            idle_timeout_ms: number;
            /** Format: int32 */
            max_concurrency: number;
            /** Format: int32 */
            min_interval_ms: number;
            proxy_url?: string | null;
            /** Format: int32 */
            rate_limit_retries: number;
            /** Format: int32 */
            request_timeout_ms: number;
        };
        LlmOutput: {
            content: components["schemas"]["LlmContent"][];
            finish_reason?: string | null;
            /** Format: int32 */
            index: number;
        };
        LlmParameterSpec: {
            choices: string[];
            description: string;
            evidence: string;
            group: string;
            key: string;
            label: string;
            /** Format: double */
            maximum?: number | null;
            /** Format: double */
            minimum?: number | null;
            support: components["schemas"]["LlmSupport"];
            value_type: string;
        };
        LlmParameters: {
            items: components["schemas"]["LlmParameterSpec"][];
        };
        LlmPrepared: {
            native_request: unknown;
            snapshot: components["schemas"]["LlmInvocationSnapshot"];
        };
        LlmPreset: {
            config: components["schemas"]["LlmPresetConfig"];
            id: string;
            /** Format: int64 */
            revision: number;
        };
        LlmPresetConfig: {
            name: string;
            parameters?: {
                [key: string]: unknown;
            };
            protocol: components["schemas"]["LlmProtocol"];
        };
        LlmPresets: {
            items: components["schemas"]["LlmPreset"][];
        };
        /** @enum {string} */
        LlmProtocol: "openai_chat" | "openai_responses" | "gemini";
        /** @enum {string} */
        LlmProviderKind: "openai_compatible" | "openai" | "openrouter" | "gemini";
        LlmProviderView: {
            config: components["schemas"]["LlmConnectionConfig"];
            credential_set: boolean;
            id: string;
            /** Format: int64 */
            revision: number;
        };
        LlmProviders: {
            items: components["schemas"]["LlmProviderView"][];
        };
        LlmResponse: {
            model?: string | null;
            outputs: components["schemas"]["LlmOutput"][];
            provider_request_id?: string | null;
            response_id?: string | null;
            snapshot: components["schemas"]["LlmInvocationSnapshot"];
            usage: components["schemas"]["LlmUsage"];
        };
        LlmRevision: {
            /** Format: int64 */
            expected_revision: number;
        };
        /** @enum {string} */
        LlmRole: "system" | "developer" | "user" | "assistant" | "tool";
        /** @enum {string} */
        LlmSupport: "supported" | "unsupported" | "unknown";
        LlmSystemPrompt: {
            config: components["schemas"]["LlmSystemPromptConfig"];
            id: string;
            /** Format: int64 */
            revision: number;
        };
        LlmSystemPromptConfig: {
            description?: string;
            name: string;
            /** @description Literal system instructions, preserved without trimming or variable substitution. */
            text: string;
        };
        LlmSystemPrompts: {
            items: components["schemas"]["LlmSystemPrompt"][];
        };
        LlmTool: {
            description: string;
            name: string;
            parameters: unknown;
            strict: boolean;
        };
        LlmUsage: {
            /** Format: int64 */
            cached_input_tokens?: number | null;
            /** Format: int64 */
            input_tokens?: number | null;
            /** Format: int64 */
            output_tokens?: number | null;
            /** Format: int64 */
            reasoning_tokens?: number | null;
            /** Format: int64 */
            total_tokens?: number | null;
        };
        ManagedJob: {
            job: components["schemas"]["Job"];
            object: components["schemas"]["ManagedObject"];
            result_available: boolean;
        };
        ManagedJobPage: {
            items: components["schemas"]["ManagedJob"][];
            next_cursor?: string | null;
        };
        ManagedObject: {
            archived: boolean;
            bytes?: string | null;
            /** Format: int64 */
            count?: number | null;
            created_at?: string | null;
            id: string;
            kind: components["schemas"]["ObjectKind"];
            name: string;
            notes: string;
            /** Format: int64 */
            revision: number;
            state: string;
            subtype?: string | null;
            updated_at?: string | null;
        };
        MaybeDraft: {
            draft?: null | components["schemas"]["Draft"];
        };
        MaybePreference: {
            preference?: null | components["schemas"]["Preference"];
        };
        MemberWriteProgress: {
            /** Format: int64 */
            completed: number;
            error?: string | null;
            state: string;
            /** Format: int64 */
            total?: number | null;
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
            observation_id?: string | null;
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
        ObjectAction: {
            action: components["schemas"]["ObjectActionKind"];
            /** Format: int64 */
            expected_revision: number;
        };
        /** @enum {string} */
        ObjectActionKind: "remove" | "archive" | "unarchive" | "reconnect";
        ObjectDetails: {
            can_remove: boolean;
            incoming: components["schemas"]["ObjectLink"][];
            incoming_cursor?: string | null;
            /** Format: int64 */
            incoming_total: number;
            object: components["schemas"]["ManagedObject"];
            outgoing: components["schemas"]["ObjectLink"][];
            outgoing_cursor?: string | null;
            /** Format: int64 */
            outgoing_total: number;
            paths: string[];
            provenance: unknown;
            remove_reason?: string | null;
            run?: null | components["schemas"]["OperatorRun"];
        };
        /** @enum {string} */
        ObjectKind: "project" | "source" | "workset" | "artifact" | "query" | "job" | "query_result" | "selection" | "selection_history";
        ObjectLink: {
            blocking: boolean;
            id: string;
            kind: components["schemas"]["ObjectKind"];
            name: string;
            relation: string;
        };
        ObjectLinkPage: {
            items: components["schemas"]["ObjectLink"][];
            next_cursor?: string | null;
            /** Format: int64 */
            total: number;
        };
        ObjectPage: {
            items: components["schemas"]["ManagedObject"][];
            next_cursor?: string | null;
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
        PresetPage: {
            items: components["schemas"]["ToolPreset"][];
            next_cursor?: string | null;
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
        ProjectCacheInventory: {
            cleanups: components["schemas"]["CacheCleanup"][];
            member_path: string;
            members: components["schemas"]["CacheMemberItem"][];
            next_cursor?: string | null;
            project_id: string;
            ranked_indexes: components["schemas"]["CacheRankedItem"][];
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
            ranked_index_builds: number;
            ranked_index_bytes: string;
            /** Format: int64 */
            ranked_index_reuses: number;
            /** Format: int64 */
            ranked_indexes: number;
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
        RankedScope: {
            artifact_id: string;
            artifact_name: string;
            /** Format: int64 */
            count: number;
            current_rating_filter: boolean;
            saved_filter: components["schemas"]["RankingFilter"];
            /** Format: int32 */
            schema_version: number;
            view_key: string;
            workset_id: string;
        };
        RankingBasis: {
            /** Format: int32 */
            index: number;
            result_id?: string | null;
            spec: components["schemas"]["QuerySpec"];
        };
        RankingBrowseInfo: {
            ranking?: null | components["schemas"]["RankedScope"];
        };
        RankingBrowseLease: {
            lease_id: string;
            release?: boolean;
            scope: components["schemas"]["ScopeRef"];
        };
        RankingBrowseRequest: {
            cursor?: string | null;
            descending?: boolean;
            limit?: number | null;
            order?: null | components["schemas"]["RankingOrder"];
            scope: components["schemas"]["ScopeRef"];
            /** @description Exact frozen Danbooru post ID; the located row is included as the first item. */
            start_post_id?: string | null;
            /**
             * @description Positive, one-based position in the current scope and viewing direction.
             *     With start_rating, this is the original rank within that frozen Rating instead.
             *     Mutually exclusive with start_post_id; the located member is included.
             */
            start_rank?: string | null;
            /**
             * @description Frozen scoring Rating (g, s, q, e). Requires start_rank and a ranking order other than input.
             *     Only locates a member; it does not filter or change the scope.
             */
            start_rating?: string | null;
        };
        RankingCount: {
            /** Format: int64 */
            count?: number | null;
            /** Format: int64 */
            scanned: number;
            /** Format: int64 */
            total: number;
        };
        RankingCountRequest: {
            filter: components["schemas"]["RankingFilter"];
        };
        RankingDuplicateEvidence: {
            counts_clamped: boolean;
            heat: components["schemas"]["RankingObservation"][];
            metadata: components["schemas"]["RankingObservation"];
            /** Format: int64 */
            omitted_posts: number;
            partial_counts: boolean;
            policy: components["schemas"]["DuplicateHeat"];
            /** Format: int64 */
            post_count: number;
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
            evidence?: null | components["schemas"]["RankingDuplicateEvidence"];
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
            tags?: string | null;
            tags_known: boolean;
            time_quality: string;
            up_score?: string | null;
            updated_at_us?: string | null;
        };
        /** @enum {string} */
        RankingMode: "rank" | "select";
        RankingObservation: {
            created_at_us?: string | null;
            down_score?: string | null;
            fav_count?: string | null;
            is_deleted?: boolean | null;
            observation_id: string;
            observed_at_us?: string | null;
            post_id?: string | null;
            rating?: string | null;
            record_id: string;
            score?: string | null;
            time_quality: string;
            up_score?: string | null;
            updated_at_us?: string | null;
        };
        /** @enum {string} */
        RankingOrder: "main" | "rescue" | "input" | "direct" | "fused";
        RankingPage: {
            artifact_id: string;
            /** Format: int64 */
            count?: number | null;
            items: components["schemas"]["RankingRow"][];
            next_cursor?: string | null;
            preparing?: string | null;
            scan?: null | components["schemas"]["BrowseScan"];
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
            duplicate_heat?: null | components["schemas"]["DuplicateHeat"];
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
            v2?: null | components["schemas"]["RankingV2Parameters"];
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
            v2?: null | components["schemas"]["RankingV2Summary"];
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
            v2?: null | components["schemas"]["RankingV2Scores"];
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
        RankingV2Parameters: {
            /** Format: int32 */
            audit: number;
            /** Format: double */
            comic_penalty: number;
            /** Format: int32 */
            direct_rescue: number;
            /** Format: int32 */
            era_rescue: number;
            eras: components["schemas"]["EraPreference"][];
            /** Format: int32 */
            feather_days: number;
            /** Format: int32 */
            keep_per_mille: number;
            /** Format: int32 */
            minimum_effective: number;
            profiles: {
                [key: string]: components["schemas"]["RatingProfile"];
            };
            strict_era_targets: boolean;
        };
        RankingV2Scores: {
            /** Format: int32 */
            created_year?: number | null;
            /** Format: double */
            direct_percentile: number;
            /** Format: int64 */
            direct_rank: number;
            /** Format: double */
            direct_raw: number;
            /** Format: double */
            effective_count: number;
            /** Format: double */
            era_bonus: number;
            era_fallback: boolean;
            /** Format: double */
            era_percentile: number;
            /** Format: double */
            era_raw: number;
            /** Format: int64 */
            fused_rank: number;
            /** Format: double */
            fused_score: number;
            layout_protected: boolean;
            /** Format: double */
            new_percentile: number;
            /** Format: int32 */
            new_period?: number | null;
            /** Format: double */
            new_weight: number;
            /** Format: double */
            old_percentile: number;
            /** Format: int32 */
            old_period?: number | null;
            /**
             * Format: int32
             * @description 0 none/main, 1 direct-only rescue, 2 era-only rescue, 3 random audit.
             */
            selection_reason: number;
            /** Format: int32 */
            type_hints: number;
            /** Format: double */
            type_penalty: number;
            /** Format: double */
            year_percentile: number;
            /** Format: int64 */
            year_rank: number;
        };
        RankingV2Summary: {
            /** Format: int64 */
            audit_selected: number;
            /** Format: int64 */
            direct_rescued: number;
            /** Format: int64 */
            era_rescued: number;
            /** Format: int64 */
            era_target_shortfall: number;
            /** Format: int64 */
            fallback_count: number;
            /** @description This release has no visual bridge calibration or inferred aesthetic probabilities. */
            metadata_only: boolean;
            /** Format: int64 */
            protected_count: number;
            protection_shortfall: number[];
            /** Format: int64 */
            type_penalized: number;
            years: components["schemas"]["RankingYearSummary"][];
        };
        RankingWorksetRequest: {
            filter?: components["schemas"]["RankingFilter"];
            idempotency_key: string;
            name: string;
        };
        RankingYearSummary: {
            /** Format: int64 */
            count: number;
            /** Format: int64 */
            fallback: number;
            /** Format: int64 */
            penalized: number;
            /** Format: int64 */
            protected: number;
            /** Format: int64 */
            selected: number;
            /** Format: int64 */
            top_count: number;
            /** Format: int32 */
            year?: number | null;
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
        RatingProfile: {
            /** Format: double */
            era_weight: number;
            /** Format: double */
            time_down: number;
            /** Format: double */
            time_up: number;
            /** Format: double */
            vote_weight: number;
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
        RevealObject: {
            file_index?: number;
            open?: boolean;
        };
        RevealedLocation: {
            opened: boolean;
            path: string;
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
        SaveLlmModel: {
            config: components["schemas"]["LlmModelConfig"];
            /** Format: int64 */
            expected_revision: number;
            id?: string | null;
            provider_id: string;
        };
        SaveLlmPreset: {
            config: components["schemas"]["LlmPresetConfig"];
            /** Format: int64 */
            expected_revision: number;
            id?: string | null;
        };
        SaveLlmProvider: {
            api_key?: string | null;
            clear_credential?: boolean;
            config: components["schemas"]["LlmConnectionConfig"];
            /** Format: int64 */
            expected_revision: number;
            id?: string | null;
        };
        SaveLlmSystemPrompt: {
            config: components["schemas"]["LlmSystemPromptConfig"];
            /** Format: int64 */
            expected_revision: number;
            id?: string | null;
        };
        SaveQuery: {
            /** Format: int64 */
            expected_revision?: number | null;
            name: string;
            spec: components["schemas"]["QuerySpec"];
        };
        SaveToolPreset: {
            /** Format: int64 */
            expected_revision: number;
            id?: string | null;
            name: string;
            notes: string;
            run: components["schemas"]["OperatorRun"];
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
        SelectionMembers: {
            /** Format: int64 */
            revision: number;
            selected: boolean[];
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
            maintenance: components["schemas"]["CacheMaintenance"];
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
        ToolPreset: {
            created_at: string;
            id: string;
            name: string;
            notes: string;
            /** Format: int64 */
            revision: number;
            run: components["schemas"]["OperatorRun"];
            updated_at: string;
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
    cache_projects: {
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
                    "application/json": components["schemas"]["CacheProjects"];
                };
            };
        };
    };
    project_cache_inventory: {
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
                    "application/json": components["schemas"]["ProjectCacheInventory"];
                };
            };
        };
    };
    release_project_cache_member: {
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
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
    retain_project_cache_member: {
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
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
    release_project_ranked_index: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
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
                    "application/json": components["schemas"]["OkResponse"];
                };
            };
        };
    };
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
    llm_generate: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["LlmInvocationRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LlmResponse"];
                };
            };
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LlmFailure"];
                };
            };
        };
    };
    llm_cancel: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
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
    llm_save_model: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SaveLlmModel"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LlmModel"];
                };
            };
        };
    };
    llm_model_parameters: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
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
                    "application/json": components["schemas"]["LlmParameters"];
                };
            };
        };
    };
    llm_remove_model: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["LlmRevision"];
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
    llm_parameters: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                protocol: components["schemas"]["LlmProtocol"];
                kind: components["schemas"]["LlmProviderKind"];
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
                    "application/json": components["schemas"]["LlmParameters"];
                };
            };
        };
    };
    llm_prepare: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["LlmInvocationRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LlmPrepared"];
                };
            };
        };
    };
    llm_presets: {
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
                    "application/json": components["schemas"]["LlmPresets"];
                };
            };
        };
    };
    llm_save_preset: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SaveLlmPreset"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LlmPreset"];
                };
            };
        };
    };
    llm_remove_preset: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["LlmRevision"];
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
    llm_providers: {
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
                    "application/json": components["schemas"]["LlmProviders"];
                };
            };
        };
    };
    llm_save_provider: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SaveLlmProvider"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LlmProviderView"];
                };
            };
        };
    };
    llm_catalog: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
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
                    "application/json": components["schemas"]["LlmCatalogStatus"];
                };
            };
        };
    };
    llm_refresh_catalog: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["DiscoverLlmModels"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LlmCatalog"];
                };
            };
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LlmFailure"];
                };
            };
        };
    };
    llm_models: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
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
                    "application/json": components["schemas"]["LlmModels"];
                };
            };
        };
    };
    llm_remove_provider: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["LlmRevision"];
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
    llm_stream: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["LlmInvocationRequest"];
            };
        };
        responses: {
            /** @description SSE data contains LlmEvent; completion or failure is required */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "text/event-stream": components["schemas"]["LlmEvent"];
                };
            };
        };
    };
    llm_system_prompts: {
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
                    "application/json": components["schemas"]["LlmSystemPrompts"];
                };
            };
        };
    };
    llm_save_system_prompt: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["SaveLlmSystemPrompt"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["LlmSystemPrompt"];
                };
            };
        };
    };
    llm_system_prompt: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
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
                    "application/json": components["schemas"]["LlmSystemPrompt"];
                };
            };
        };
    };
    llm_remove_system_prompt: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["LlmRevision"];
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
    aesthetic_experiments: {
        parameters: {
            query?: {
                after?: string;
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
                    "application/json": components["schemas"]["AestheticExperiments"];
                };
            };
        };
    };
    aesthetic_experiment_create: {
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
                "application/json": components["schemas"]["AestheticExperimentCreate"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AestheticExperiment"];
                };
            };
        };
    };
    aesthetic_experiment: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
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
                    "application/json": components["schemas"]["AestheticExperiment"];
                };
            };
        };
    };
    aesthetic_experiment_run: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
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
                    "application/json": components["schemas"]["AestheticAnalysisJobs"];
                };
            };
        };
    };
    aesthetic_analysis_jobs: {
        parameters: {
            query?: {
                after?: string;
                limit?: number;
                experiment_id?: string;
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
                    "application/json": components["schemas"]["AestheticAnalysisJobs"];
                };
            };
        };
    };
    aesthetic_analysis_create: {
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
                "application/json": components["schemas"]["AestheticAnalysisCreate"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AestheticAnalysisJob"];
                };
            };
        };
    };
    aesthetic_analysis_job: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
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
                    "application/json": components["schemas"]["AestheticAnalysisJob"];
                };
            };
        };
    };
    aesthetic_comparison_rows: {
        parameters: {
            query?: {
                after?: string;
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
                id: string;
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
                    "application/json": components["schemas"]["AestheticComparisonRows"];
                };
            };
        };
    };
    aesthetic_analysis_control: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AestheticAnalysisControl"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AestheticAnalysisJob"];
                };
            };
        };
    };
    aesthetic_review_create: {
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
                "application/json": components["schemas"]["AestheticReviewCreate"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AestheticReview"];
                };
            };
        };
    };
    aesthetic_snapshot: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
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
                    "application/json": components["schemas"]["AestheticAnalysisJob"];
                };
            };
        };
    };
    aesthetic_ranking_candidate: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
                ordinal: number;
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
                    "application/json": components["schemas"]["AestheticRankingRow"];
                };
            };
        };
    };
    aesthetic_reviews: {
        parameters: {
            query?: {
                after?: string;
                /** @description Optional candidate ordinal; returns newest first, with after as an exclusive upper sequence bound. */
                ordinal?: number;
            };
            header?: never;
            path: {
                project_id: string;
                id: string;
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
                    "application/json": components["schemas"]["AestheticReviews"];
                };
            };
        };
    };
    aesthetic_ranking_rows: {
        parameters: {
            query?: {
                after?: string;
                limit?: number;
                rating?: string;
            };
            header?: never;
            path: {
                project_id: string;
                id: string;
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
                    "application/json": components["schemas"]["AestheticRankingRows"];
                };
            };
        };
    };
    aesthetic_ranking_select: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AestheticRankingQuery"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AestheticRankingSelection"];
                };
            };
        };
    };
    aesthetic_backup: {
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
                    "application/json": components["schemas"]["AestheticBackup"];
                };
            };
        };
    };
    aesthetic_capabilities: {
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
                    "application/json": components["schemas"]["AestheticCapabilities"];
                };
            };
        };
    };
    aesthetic_abandon_creation: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
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
    aesthetic_metrics: {
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
                    "application/json": components["schemas"]["AestheticMetrics"];
                };
            };
        };
    };
    aesthetic_preflight: {
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
                "application/json": components["schemas"]["AestheticCreate"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AestheticPreflight"];
                };
            };
        };
    };
    aesthetic_stages: {
        parameters: {
            query?: {
                after?: string;
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
                    "application/json": components["schemas"]["AestheticStages"];
                };
            };
        };
    };
    aesthetic_create: {
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
                "application/json": components["schemas"]["AestheticCreate"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AestheticStage"];
                };
            };
        };
    };
    aesthetic_stage: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
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
                    "application/json": components["schemas"]["AestheticStage"];
                };
            };
        };
    };
    aesthetic_batches: {
        parameters: {
            query?: {
                after?: string;
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
                id: string;
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
                    "application/json": components["schemas"]["AestheticBatches"];
                };
            };
        };
    };
    aesthetic_attempts: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
                batch: number;
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
                    "application/json": components["schemas"]["AestheticAttempts"];
                };
            };
        };
    };
    aesthetic_retry: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
                batch: number;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AestheticRetry"];
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
    aesthetic_candidates: {
        parameters: {
            query?: {
                after?: string;
                protected?: boolean;
            };
            header?: never;
            path: {
                project_id: string;
                id: string;
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
                    "application/json": components["schemas"]["AestheticCandidates"];
                };
            };
        };
    };
    aesthetic_decide_candidate: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
                ordinal: number;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AestheticCandidateDecision"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AestheticCandidate"];
                };
            };
        };
    };
    aesthetic_control: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["AestheticControl"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AestheticStage"];
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
    ranking_count: {
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
                "application/json": components["schemas"]["RankingCountRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RankingCount"];
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
    ranking_row: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                artifact_id: string;
                ordinal: number;
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
                    "application/json": components["schemas"]["RankingRow"];
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
    asset_summaries: {
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
                "application/json": components["schemas"]["AssetKeysRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["AssetSummaries"];
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
    jobs: {
        parameters: {
            query?: {
                search?: string;
                state?: string;
                include_archived?: boolean;
                order?: string;
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
                    "application/json": components["schemas"]["ManagedJobPage"];
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
    member_write_progress: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                operation_id: string;
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
                    "application/json": components["schemas"]["MemberWriteProgress"];
                };
            };
        };
    };
    cancel_member_write: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                operation_id: string;
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
    list: {
        parameters: {
            query?: {
                search?: string;
                order?: string;
                state?: string;
                subtype?: string;
                include_archived?: boolean;
                cursor?: string;
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
                kind: string;
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
                    "application/json": components["schemas"]["ObjectPage"];
                };
            };
        };
    };
    read: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                kind: string;
                object_id: string;
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
                    "application/json": components["schemas"]["ObjectDetails"];
                };
            };
        };
    };
    edit: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                kind: string;
                object_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["EditObject"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ManagedObject"];
                };
            };
        };
    };
    action: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                kind: string;
                object_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ObjectAction"];
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
    links: {
        parameters: {
            query?: {
                incoming?: boolean;
                cursor?: string;
                limit?: number;
            };
            header?: never;
            path: {
                project_id: string;
                kind: string;
                object_id: string;
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
                    "application/json": components["schemas"]["ObjectLinkPage"];
                };
            };
        };
    };
    reveal: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                kind: string;
                object_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["RevealObject"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["RevealedLocation"];
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
    presets: {
        parameters: {
            query: {
                operator_id: string;
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
                    "application/json": components["schemas"]["PresetPage"];
                };
            };
        };
    };
    save_preset: {
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
                "application/json": components["schemas"]["SaveToolPreset"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ToolPreset"];
                };
            };
        };
    };
    delete_preset: {
        parameters: {
            query?: never;
            header?: never;
            path: {
                project_id: string;
                preset_id: string;
            };
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["DeleteToolPreset"];
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
    ranking_browse_info: {
        parameters: {
            query?: {
                collection_id?: string;
                result_id?: string;
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
                    "application/json": components["schemas"]["RankingBrowseInfo"];
                };
            };
        };
    };
    ranking_browse_assets: {
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
                "application/json": components["schemas"]["RankingBrowseRequest"];
            };
        };
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
    ranking_scope_lease: {
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
                "application/json": components["schemas"]["RankingBrowseLease"];
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
    history: {
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
                    "application/json": components["schemas"]["HistoryStatus"];
                };
            };
        };
    };
    restore: {
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
                "application/json": components["schemas"]["HistoryAction"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["HistoryStatus"];
                };
            };
        };
    };
    selection_members: {
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
                "application/json": components["schemas"]["AssetKeysRequest"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["SelectionMembers"];
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
                observation_id?: string;
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
    editing: {
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
                    "application/json": components["schemas"]["EditingSettings"];
                };
            };
        };
    };
    configure_editing: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["ConfigureEditing"];
            };
        };
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["EditingSettings"];
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
