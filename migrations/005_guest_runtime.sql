CREATE TABLE function_summaries(
 function TEXT PRIMARY KEY REFERENCES ir_functions(function) ON DELETE CASCADE,
 version INTEGER NOT NULL,
 call_model_version INTEGER NOT NULL,
 ir_hash TEXT NOT NULL,
 dependency_hash TEXT NOT NULL,
 artifact TEXT NOT NULL
);
CREATE TABLE call_contracts(
 function TEXT NOT NULL REFERENCES function_summaries(function) ON DELETE CASCADE,
 site INTEGER NOT NULL,
 version INTEGER NOT NULL,
 certainty TEXT NOT NULL,
 target_kind TEXT NOT NULL,
 artifact TEXT NOT NULL,
 PRIMARY KEY(function,site)
);
CREATE TABLE runtime_imports(
 function TEXT NOT NULL,
 site INTEGER NOT NULL,
 library TEXT NOT NULL,
 symbol TEXT NOT NULL,
 contract_status TEXT NOT NULL,
 handler_status TEXT NOT NULL,
 PRIMARY KEY(function,site),
 FOREIGN KEY(function,site) REFERENCES call_contracts(function,site) ON DELETE CASCADE
);
