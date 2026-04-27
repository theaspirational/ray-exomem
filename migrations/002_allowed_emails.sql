CREATE TABLE IF NOT EXISTS allowed_emails (
    email TEXT PRIMARY KEY,
    alias TEXT NOT NULL DEFAULT ''
);
