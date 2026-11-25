CREATE TABLE IF NOT EXISTS lcad_schema (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL
) STRICT;
INSERT INTO lcad_schema VALUES (1, 1) ON CONFLICT(id) DO NOTHING;
CREATE TABLE IF NOT EXISTS lcad_jobs (
    id INTEGER PRIMARY KEY,
    scope TEXT NOT NULL,
    project TEXT NOT NULL,
    task_key TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation > 0),
    provider TEXT NOT NULL,
    spec TEXT NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
    state TEXT NOT NULL DEFAULT 'admitted' CHECK (
        state IN ('admitted', 'running', 'computed', 'published', 'superseded', 'failed', 'cancelled')
    ),
    UNIQUE (scope, task_key, generation)
) STRICT;
CREATE INDEX IF NOT EXISTS lcad_ready ON lcad_jobs(provider, id) WHERE state = 'admitted';
CREATE TABLE IF NOT EXISTS lcad_dependencies (
    job INTEGER NOT NULL REFERENCES lcad_jobs(id),
    slot TEXT NOT NULL,
    dependency INTEGER NOT NULL REFERENCES lcad_jobs(id),
    PRIMARY KEY (job, slot)
) STRICT;
CREATE TABLE IF NOT EXISTS lcad_intents (
    scope TEXT NOT NULL,
    intent_key TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation > 0),
    job INTEGER NOT NULL REFERENCES lcad_jobs(id),
    PRIMARY KEY (scope, intent_key, generation)
) STRICT;
CREATE TABLE IF NOT EXISTS lcad_attempts (
    job INTEGER NOT NULL REFERENCES lcad_jobs(id),
    number INTEGER NOT NULL CHECK (number > 0),
    state TEXT NOT NULL,
    artifact TEXT,
    publication TEXT,
    failure TEXT,
    PRIMARY KEY (job, number)
) STRICT;
CREATE TABLE IF NOT EXISTS lcad_source_heads (
    project TEXT NOT NULL,
    document TEXT NOT NULL,
    pin TEXT NOT NULL,
    PRIMARY KEY (project, document)
) STRICT;
CREATE TABLE IF NOT EXISTS lcad_slots (
    project TEXT NOT NULL,
    slot TEXT NOT NULL,
    generation TEXT NOT NULL,
    job INTEGER NOT NULL REFERENCES lcad_jobs(id),
    PRIMARY KEY (project, slot)
) STRICT;
