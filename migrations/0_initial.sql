CREATE TABLE files (
    folder_id INTEGER NOT NULL,
    path TEXT NOT NULL,
    local_hash BLOB,
    local_last_modified INTEGER,
    global_hash BLOB NOT NULL,
    global_last_modified INTEGER NOT NULL,
    global_peer TEXT NOT NULL,
    PRIMARY KEY (folder_id, path)
);
