-- Config globale (clé/valeur)
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Cibles à snapshotter
CREATE TABLE IF NOT EXISTS targets (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    url        TEXT NOT NULL UNIQUE,
    enabled    INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Index des snapshots
CREATE TABLE IF NOT EXISTS snapshots (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    target_id    INTEGER NOT NULL REFERENCES targets(id) ON DELETE CASCADE,
    url          TEXT NOT NULL,
    taken_at     TEXT NOT NULL,
    size_bytes   INTEGER NOT NULL,
    sha256       TEXT NOT NULL,
    local_path   TEXT NOT NULL,
    drive_file_id TEXT,
    status       TEXT NOT NULL,  -- 'ok', 'fetch_error', 'upload_error'
    error        TEXT
);

CREATE INDEX IF NOT EXISTS idx_snapshots_target_taken
    ON snapshots(target_id, taken_at DESC);

CREATE INDEX IF NOT EXISTS idx_snapshots_sha
    ON snapshots(target_id, sha256);
