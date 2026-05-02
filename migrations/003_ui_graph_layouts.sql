CREATE TABLE IF NOT EXISTS ui_graph_layouts (
    user_email TEXT        NOT NULL REFERENCES users(email) ON DELETE CASCADE,
    scope      TEXT        NOT NULL,
    layout     JSONB       NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_email, scope)
);
