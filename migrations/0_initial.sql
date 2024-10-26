CREATE TABLE files (
    folder_id INTEGER NOT NULL,
    relative_path TEXT NOT NULL,
    local_hash BLOB NOT NULL,
    global_hash BLOB NOT NULL,
    last_modified INTEGER,
    PRIMARY KEY (folder_id, relative_path)
);
