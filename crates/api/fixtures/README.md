# API Fixtures

This directory is for deterministic API-side fixtures only.

Runtime data must not be committed here. Live NAV samples, order journals,
audit logs, cache databases, and local SQLite files belong in the configured
runtime data directory, not in this fixture tree.

SQLite fixtures must live in a named subdirectory with its own README that
declares the authoritative migration, schema identity, fixture purpose, and
copy-to-tempdir test rule.
