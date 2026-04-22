-- Auth tables
CREATE TABLE IF NOT EXISTS users (
    email        TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    provider     TEXT NOT NULL,
    role         TEXT NOT NULL DEFAULT 'regular',
    active       BOOLEAN NOT NULL DEFAULT true,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_login   TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    email      TEXT NOT NULL REFERENCES users(email),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_email ON sessions(email);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);

CREATE TABLE IF NOT EXISTS api_keys (
    key_id     TEXT PRIMARY KEY,
    key_hash   TEXT NOT NULL UNIQUE,
    email      TEXT NOT NULL REFERENCES users(email),
    label      TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_api_keys_email ON api_keys(email);

CREATE TABLE IF NOT EXISTS shares (
    share_id      TEXT PRIMARY KEY,
    owner_email   TEXT NOT NULL REFERENCES users(email),
    path          TEXT NOT NULL,
    grantee_email TEXT NOT NULL REFERENCES users(email),
    permission    TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_shares_grantee ON shares(grantee_email);
CREATE INDEX IF NOT EXISTS idx_shares_owner ON shares(owner_email);
CREATE INDEX IF NOT EXISTS idx_shares_path ON shares(path);

CREATE TABLE IF NOT EXISTS allowed_domains (
    domain TEXT PRIMARY KEY
);
