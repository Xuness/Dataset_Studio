-- Query v2 adds typed value sets and project-scope references inside spec_json.
-- Advance the project compatibility marker before storing these definitions:
-- older engines must reject v5 instead of misreading or rewriting v2 queries.
-- Existing tables and records require no physical transformation.
SELECT 1;
