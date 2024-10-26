CREATE TABLE files (
    path TEXT NOT NULL PRIMARY KEY,
    local_hash BLOB NOT NULL,
    global_hash BLOB NOT NULL,
    last_modified INTEGER NOT NULL
);
