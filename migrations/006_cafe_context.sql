-- Metadata only. Image bytes are always rederived from the authorized module.
CREATE TABLE analysis_products (
    kind TEXT NOT NULL,
    subject TEXT NOT NULL,
    module TEXT NOT NULL REFERENCES modules(id) ON DELETE CASCADE,
    cache_key TEXT NOT NULL,
    versions TEXT NOT NULL,
    artifact TEXT NOT NULL,
    PRIMARY KEY(kind, subject)
);
CREATE TABLE function_validation_runs (
    id TEXT PRIMARY KEY,
    function TEXT NOT NULL REFERENCES ir_functions(function) ON DELETE CASCADE,
    image_hash TEXT NOT NULL,
    context_version INTEGER NOT NULL,
    domain TEXT NOT NULL,
    status TEXT NOT NULL,
    report_path TEXT NOT NULL,
    report_hash TEXT NOT NULL,
    summary TEXT NOT NULL
);
