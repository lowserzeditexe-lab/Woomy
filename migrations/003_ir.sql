ALTER TABLE modules ADD COLUMN ir_version INTEGER NOT NULL DEFAULT 0;
CREATE TABLE ir_functions(function TEXT PRIMARY KEY REFERENCES functions(id) ON DELETE CASCADE, ir_version INTEGER NOT NULL, module_hash TEXT NOT NULL, status TEXT NOT NULL, artifact TEXT NOT NULL);
CREATE TABLE ir_verifications(function TEXT PRIMARY KEY REFERENCES ir_functions(function) ON DELETE CASCADE, ir_version INTEGER NOT NULL, status TEXT NOT NULL, unknown_operations INTEGER NOT NULL, report TEXT NOT NULL);
