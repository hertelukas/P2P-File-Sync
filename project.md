## Hertel - P2P file sync service

### Students

- Lukas Hertel <lukas.hertel@telecom-sudparis.eu>

### Repository

https://gitlab.telecom-paris.fr/net7212/2425/project-hertel/

### Description

A Linux-based peer-to-peer file sync service, running as a daemon.
The service watches for changes in a specified folder and syncs them across computers using P2P architecture.
Initially, it will support two connections but should later support multiple peers.
Conflict resolution will favor the latest file, renaming older ones.
Similar to the existing \[Syncthing project\](<https://docs.syncthing.net/index.html>).

#### Core Features

- *File Watching and Syncing*: Watch a specific folder for file changes, see `inotify`.
  Hourly rescans are performed as well.
  Each peer keeps a SQLite database with current hashes of the files, and the global - newest - file hash.
  These then need to be updated.

- *Peer-to-Peer Communication*: Direct sync between two or more devices using a P2P architecture.

- *Conflict Resolution*: Conflicts can only occur when two files change while the peers are not in contact with each other.
  If this should be the case, the newer version overwrites older ones.
  Older files are renamed with a `.sync-conflict-<date>` suffix and are also synchronized. 

#### Daemon Service

- The program runs as a background service on Linux.
  It starts automatically and continuously monitors a folder. 

- Lightweight protocol to transfer file changes efficiently.
  (Look at splitting into blocks, updating only specific blocks, compression, reusage...) 

#### CLI Tool

- A command-line interface for configuration.
  Users can add peers by IP address and view the status of the sync.
  (Device Identifiers allow more flexibility and might be added for authenticated transfers)

- *Configuration Storage*: Store configuration data (peers, folders, etc.) in the user's `.config` directory, using TOML.

#### Open Issues

- IP addresses are a bad idea as identifier for peers, as they are not static and can be faked

- Unclear how I can identify a folder over different devices

  - A shares folder `foo` with B, and C
  - C wants to share `foo` with B
  - B needs to know that this `foo` is the same one, also shared with A 

- Not sure how the daemon service should notice changes made by the CLI tool and reacts to them
