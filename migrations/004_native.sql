CREATE TABLE codegen_artifacts(
    function TEXT PRIMARY KEY REFERENCES ir_functions(function) ON DELETE CASCADE,
    cache_key TEXT NOT NULL,
    backend_version INTEGER NOT NULL,
    native_abi_version INTEGER NOT NULL,
    status TEXT NOT NULL,
    manifest TEXT NOT NULL
);
CREATE TABLE native_builds(
    id INTEGER PRIMARY KEY,
    function TEXT NOT NULL REFERENCES ir_functions(function) ON DELETE CASCADE,
    cache_key TEXT NOT NULL,
    status TEXT NOT NULL,
    executable TEXT,
    record TEXT NOT NULL
);
CREATE INDEX native_build_identity ON native_builds(function,cache_key,id);
