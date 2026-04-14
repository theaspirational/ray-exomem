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

-- Core exom tables
CREATE TABLE IF NOT EXISTS transactions (
    id           BIGSERIAL PRIMARY KEY,
    exom_path    TEXT NOT NULL,
    tx_id        BIGINT NOT NULL,
    tx_time      TIMESTAMPTZ NOT NULL,
    user_email   TEXT,
    actor        TEXT,
    action       TEXT NOT NULL,
    refs         TEXT[] NOT NULL DEFAULT '{}',
    note         TEXT NOT NULL DEFAULT '',
    parent_tx_id BIGINT,
    branch_id    TEXT NOT NULL DEFAULT 'main',
    session      TEXT,
    UNIQUE(exom_path, tx_id)
);
CREATE INDEX IF NOT EXISTS idx_tx_exom ON transactions(exom_path);

CREATE TABLE IF NOT EXISTS facts (
    id               BIGSERIAL PRIMARY KEY,
    exom_path        TEXT NOT NULL,
    fact_id          TEXT NOT NULL,
    predicate        TEXT NOT NULL,
    value            TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL,
    created_by_tx    BIGINT NOT NULL,
    superseded_by_tx BIGINT,
    revoked_by_tx    BIGINT,
    confidence       DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    provenance       TEXT NOT NULL DEFAULT '',
    valid_from       TIMESTAMPTZ NOT NULL,
    valid_to         TIMESTAMPTZ,
    UNIQUE(exom_path, fact_id)
);
CREATE INDEX IF NOT EXISTS idx_facts_exom ON facts(exom_path);
CREATE INDEX IF NOT EXISTS idx_facts_predicate ON facts(exom_path, predicate);

CREATE TABLE IF NOT EXISTS observations (
    id          BIGSERIAL PRIMARY KEY,
    exom_path   TEXT NOT NULL,
    obs_id      TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_ref  TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL,
    confidence  DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    tx_id       BIGINT NOT NULL,
    tags        TEXT[] NOT NULL DEFAULT '{}',
    valid_from  TIMESTAMPTZ NOT NULL,
    valid_to    TIMESTAMPTZ,
    UNIQUE(exom_path, obs_id)
);
CREATE INDEX IF NOT EXISTS idx_obs_exom ON observations(exom_path);

CREATE TABLE IF NOT EXISTS beliefs (
    id            BIGSERIAL PRIMARY KEY,
    exom_path     TEXT NOT NULL,
    belief_id     TEXT NOT NULL,
    claim_text    TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'active',
    confidence    DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    supported_by  TEXT[] NOT NULL DEFAULT '{}',
    created_by_tx BIGINT NOT NULL,
    valid_from    TIMESTAMPTZ NOT NULL,
    valid_to      TIMESTAMPTZ,
    rationale     TEXT NOT NULL DEFAULT '',
    UNIQUE(exom_path, belief_id)
);
CREATE INDEX IF NOT EXISTS idx_beliefs_exom ON beliefs(exom_path);

CREATE TABLE IF NOT EXISTS branches (
    id               BIGSERIAL PRIMARY KEY,
    exom_path        TEXT NOT NULL,
    branch_id        TEXT NOT NULL,
    name             TEXT NOT NULL,
    parent_branch_id TEXT,
    created_tx_id    BIGINT NOT NULL,
    archived         BOOLEAN NOT NULL DEFAULT false,
    claimed_by       TEXT,
    UNIQUE(exom_path, branch_id)
);
CREATE INDEX IF NOT EXISTS idx_branches_exom ON branches(exom_path);
