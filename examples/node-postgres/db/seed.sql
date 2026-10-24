-- Runs exactly once, after PostgreSQL is healthy.
CREATE TABLE IF NOT EXISTS todos (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    title text NOT NULL
);

INSERT INTO todos (title)
SELECT 'Open http://api.localhost'
WHERE NOT EXISTS (SELECT 1 FROM todos);
