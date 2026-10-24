-- Runs exactly once, after PostgreSQL is healthy.
CREATE TABLE IF NOT EXISTS notes (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    body text NOT NULL
);

INSERT INTO notes (body)
SELECT 'Open http://api.localhost'
WHERE NOT EXISTS (SELECT 1 FROM notes);
