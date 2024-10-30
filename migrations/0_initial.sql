CREATE TABLE files (
    path TEXT NOT NULL PRIMARY KEY,
    local_hash BLOB,
    local_last_modified INTEGER,
    global_hash BLOB NOT NULL,
    global_last_modified INTEGER NOT NULL,
    global_peer TEXT NOT NULL
);
