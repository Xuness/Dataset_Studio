CREATE TABLE llm_providers(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,json TEXT NOT NULL);
CREATE TABLE llm_models(
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES llm_providers(id) ON DELETE RESTRICT,
    remote_model_id TEXT NOT NULL,
    protocol TEXT NOT NULL,
    revision INTEGER NOT NULL,
    json TEXT NOT NULL,
    UNIQUE(provider_id,remote_model_id,protocol)
);
CREATE INDEX llm_models_by_provider ON llm_models(provider_id,id);
CREATE TABLE llm_presets(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,json TEXT NOT NULL);
CREATE TABLE llm_catalogs(provider_id TEXT PRIMARY KEY REFERENCES llm_providers(id) ON DELETE CASCADE,json TEXT NOT NULL);
