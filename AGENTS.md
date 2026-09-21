# AGENTS.md

## Overview

A Cargo workspace containing high-level abstractions and opinionated utilities for Rust backend development. Primarily for internal use within the `alternate` organization. Builds on top of the lower-level crates in the `foundation` and `platform` workspaces.

## Layout

```
crates/
├── api/          – API utils (Axum extractors and error handling, cursor pagination, OpenAPI components)
├── http-cache/   – HTTP caching abstraction
├── middleware/   – Reusable Tower middleware
├── platform/     – Convenience trait wrappers around lower-level platform abstractions
├── postgres/     – Postgres utils (scoped transactions, DDL functions, testing client)
└── worker/       – Background worker abstraction (cron, queue)
```
