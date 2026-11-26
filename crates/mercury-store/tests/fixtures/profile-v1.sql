-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
-- SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
-- A profile database as schema version 1 left it. The text between the two markers is migration 1
-- of src/schema.rs, copied verbatim; the test in tests/migration_fixture.rs fails when they differ.
-- migration 1 begins
CREATE TABLE places (
        id            INTEGER PRIMARY KEY,
        url           TEXT NOT NULL UNIQUE,
        title         TEXT,
        visit_count   INTEGER NOT NULL DEFAULT 0,
        typed_count   INTEGER NOT NULL DEFAULT 0,
        last_visit_at INTEGER,
        frecency      INTEGER NOT NULL DEFAULT -1,
        hidden        INTEGER NOT NULL DEFAULT 0,
        url_key       TEXT NOT NULL,             -- see search::url_key
        frecency_at   INTEGER NOT NULL DEFAULT 0, -- when `frecency` was computed
        title_at      INTEGER                    -- when the visit that gave `title` happened
    ) STRICT;
    CREATE INDEX places_by_url_key  ON places(url_key);
    CREATE INDEX places_by_frecency ON places(frecency);
    CREATE INDEX places_frecency_by_id ON places(id, frecency);
    CREATE INDEX places_by_frecency_at ON places(frecency_at, frecency);

    CREATE TABLE visits (
        id         INTEGER PRIMARY KEY,
        place_id   INTEGER NOT NULL REFERENCES places(id) ON DELETE CASCADE,
        visited_at INTEGER NOT NULL,
        transition INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX visits_by_place ON visits(place_id, visited_at);
    CREATE INDEX visits_by_date  ON visits(visited_at);

    CREATE TABLE bookmarks (
        id          INTEGER PRIMARY KEY,
        guid        TEXT NOT NULL UNIQUE,
        kind        INTEGER NOT NULL,
        parent_id   INTEGER REFERENCES bookmarks(id) ON DELETE CASCADE,
        position    INTEGER NOT NULL,
        title       TEXT,
        place_id    INTEGER REFERENCES places(id),
        added_at    INTEGER NOT NULL,
        modified_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX bookmarks_by_parent ON bookmarks(parent_id, position);
    CREATE INDEX bookmarks_by_place  ON bookmarks(place_id);

    CREATE TABLE downloads (
        id             INTEGER PRIMARY KEY,
        guid           TEXT NOT NULL UNIQUE,
        url            TEXT NOT NULL,
        final_url      TEXT,
        referrer       TEXT,
        target_path    TEXT NOT NULL,
        temp_path      TEXT,
        mime_type      TEXT,
        total_bytes    INTEGER,
        received_bytes INTEGER NOT NULL DEFAULT 0,
        state          INTEGER NOT NULL,
        error          TEXT,
        etag           TEXT,
        last_modified  TEXT,
        started_at     INTEGER NOT NULL,
        ended_at       INTEGER
    ) STRICT;

    -- The page mirror indexes `url_key` (no scheme, user info or leading www.; Unicode host)
    -- rather than `url`, so that the first keystrokes of a URL are not words of every page.
    CREATE VIRTUAL TABLE places_fts USING fts5(
        url_key, title, content='places', content_rowid='id', prefix='1 2 3'
    );
    CREATE TRIGGER places_fts_ai AFTER INSERT ON places BEGIN
        INSERT INTO places_fts(rowid, url_key, title) VALUES (new.id, new.url_key, new.title);
    END;
    CREATE TRIGGER places_fts_ad AFTER DELETE ON places BEGIN
        INSERT INTO places_fts(places_fts, rowid, url_key, title)
        VALUES ('delete', old.id, old.url_key, old.title);
    END;
    CREATE TRIGGER places_fts_au AFTER UPDATE OF url_key, title ON places BEGIN
        INSERT INTO places_fts(places_fts, rowid, url_key, title)
        VALUES ('delete', old.id, old.url_key, old.title);
        INSERT INTO places_fts(rowid, url_key, title) VALUES (new.id, new.url_key, new.title);
    END;

    CREATE VIRTUAL TABLE bookmarks_fts USING fts5(
        title, content='bookmarks', content_rowid='id', prefix='1 2 3'
    );
    CREATE TRIGGER bookmarks_fts_ai AFTER INSERT ON bookmarks BEGIN
        INSERT INTO bookmarks_fts(rowid, title) VALUES (new.id, new.title);
    END;
    CREATE TRIGGER bookmarks_fts_ad AFTER DELETE ON bookmarks BEGIN
        INSERT INTO bookmarks_fts(bookmarks_fts, rowid, title) VALUES ('delete', old.id, old.title);
    END;
    CREATE TRIGGER bookmarks_fts_au AFTER UPDATE OF title ON bookmarks BEGIN
        INSERT INTO bookmarks_fts(bookmarks_fts, rowid, title) VALUES ('delete', old.id, old.title);
        INSERT INTO bookmarks_fts(rowid, title) VALUES (new.id, new.title);
    END;

    -- The roots, after the triggers so that the full-text mirror includes them.
    INSERT INTO bookmarks (guid, kind, parent_id, position, title, place_id, added_at, modified_at)
    VALUES ('toolbar_____', 2, NULL, 0, 'Bookmarks Bar', NULL, 0, 0),
           ('menu________', 2, NULL, 1, 'Other Bookmarks', NULL, 0, 0);

-- migration 1 ends
PRAGMA user_version = 1;

INSERT INTO places (id, url, title, visit_count, typed_count, last_visit_at, frecency, url_key, frecency_at, title_at)
VALUES (1, 'https://example.com/', 'Example', 2, 1, 1794700800000, 200, 'example.com/', 1794700800000, 1794700800000),
       (2, 'https://rust-lang.org/', 'Rust', 1, 0, 1794614400000, 100, 'rust-lang.org/', 1794614400000, 1794614400000),
       (3, 'https://mercury.example/docs', NULL, 0, 0, NULL, -1, 'mercury.example/docs', 0, NULL);
INSERT INTO visits (place_id, visited_at, transition)
VALUES (1, 1794614400000, 1), (1, 1794700800000, 2), (2, 1794614400000, 1);
INSERT INTO bookmarks (guid, kind, parent_id, position, title, place_id, added_at, modified_at)
VALUES ('bm-example-1', 1, (SELECT id FROM bookmarks WHERE guid = 'toolbar_____'), 0, 'Example', 1, 1794614400000, 1794614400000),
       ('bm-rust-0001', 1, (SELECT id FROM bookmarks WHERE guid = 'menu________'), 0, 'Rust', 2, 1794614400000, 1794614400000);
INSERT INTO downloads (guid, url, target_path, received_bytes, state, started_at, ended_at)
VALUES ('dl-0000001', 'https://example.com/file.zip', '/Users/me/Downloads/file.zip', 1000, 1, 1794614400000, 1794614500000);
