-- One cell of the dish. Every cell is its own fleet file; the control
-- database gets these tables too and never uses them.
--
-- `tick` is the conductor tick that wrote the row. An upsert never replaces a
-- newer row, and a delete only removes rows older than the tick that moved
-- the agent (docs/SPEC.md §4.4).
CREATE TABLE agents (
    id INTEGER PRIMARY KEY,
    x REAL NOT NULL,
    y REAL NOT NULL,
    vx REAL NOT NULL,
    vy REAL NOT NULL,
    tick INTEGER NOT NULL
);

-- The cell's 8 × 8 texels of trail, one byte each, row by row. One row.
CREATE TABLE trail (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    tick INTEGER NOT NULL,
    data BLOB NOT NULL
);
