# Portfolio NAV Fixtures

No runtime NAV database is committed here.

Authoritative schema:

- Migration: `crates/api/migrations/20260605_portfolio_nav.sql`
- Runtime table: `portfolio_nav_samples`
- Metadata table: `portfolio_nav_meta`
- Schema identity: `nav_persist::nav_schema_hash()` (`fnv1a64:<16 hex>`)

Fixture rules:

- Store only deterministic, sanitized NAV fixtures in this directory.
- Never commit live `portfolio_nav.sqlite` files from the runtime data dir.
- Tests must copy fixture files to a temporary directory before opening them,
  because NAV load may prune old samples.
- Any SQLite fixture must be paired with metadata that records the migration
  path and schema identity used to create it.
