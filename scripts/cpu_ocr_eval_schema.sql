-- Dedicated OCR evidence store. Original inputs and raw outputs are retained as BLOBs.
PRAGMA foreign_keys = ON;
PRAGMA user_version = 1;

CREATE TABLE artifacts (
    id INTEGER PRIMARY KEY,
    sha256 TEXT NOT NULL UNIQUE CHECK(length(sha256) = 64),
    byte_count INTEGER NOT NULL CHECK(byte_count >= 0),
    media_type TEXT NOT NULL,
    content BLOB NOT NULL CHECK(length(content) = byte_count)
);
CREATE TABLE sources (
    id INTEGER PRIMARY KEY,
    uri TEXT NOT NULL,
    license TEXT NOT NULL,
    license_url TEXT,
    description TEXT NOT NULL,
    dataset_name TEXT,
    dataset_version TEXT,
    repository_url TEXT,
    repository_commit TEXT,
    repository_tree TEXT,
    repository_checksum TEXT,
    repository_checksum_method TEXT,
    repository_commit_at TEXT,
    fixture_identifier TEXT NOT NULL,
    fixture_commit TEXT,
    fixture_added_at TEXT,
    selection_note TEXT,
    published_at TEXT,
    retrieved_at TEXT NOT NULL,
    doi TEXT,
    original_artifact_id INTEGER NOT NULL REFERENCES artifacts(id)
);
CREATE TABLE source_assets (
    source_id INTEGER NOT NULL REFERENCES sources(id),
    artifact_id INTEGER NOT NULL REFERENCES artifacts(id),
    role TEXT NOT NULL,
    uri TEXT NOT NULL,
    PRIMARY KEY(source_id, artifact_id, role)
);
CREATE TABLE fixtures (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    source_id INTEGER NOT NULL REFERENCES sources(id),
    kind TEXT NOT NULL,
    expected_outcome TEXT NOT NULL CHECK(expected_outcome IN ('success', 'failed')),
    generator_json TEXT NOT NULL CHECK(json_valid(generator_json))
);
CREATE TABLE fixture_pages (
    id INTEGER PRIMARY KEY,
    fixture_id INTEGER NOT NULL REFERENCES fixtures(id),
    page_number INTEGER NOT NULL CHECK(page_number > 0),
    ground_truth TEXT NOT NULL,
    ground_truth_artifact_id INTEGER NOT NULL REFERENCES artifacts(id),
    ground_truth_commit TEXT,
    ground_truth_added_at TEXT,
    width_pixels INTEGER NOT NULL,
    height_pixels INTEGER NOT NULL,
    dpi INTEGER,
    UNIQUE(fixture_id, page_number)
);
CREATE TABLE truth_regions (
    id INTEGER PRIMARY KEY,
    fixture_page_id INTEGER NOT NULL REFERENCES fixture_pages(id),
    ordinal INTEGER NOT NULL,
    text TEXT NOT NULL,
    x REAL NOT NULL, y REAL NOT NULL, width REAL NOT NULL, height REAL NOT NULL,
    coordinate_space TEXT NOT NULL,
    precision_note TEXT NOT NULL,
    UNIQUE(fixture_page_id, ordinal)
);
CREATE TABLE environments (
    id INTEGER PRIMARY KEY,
    captured_at TEXT NOT NULL,
    executor TEXT NOT NULL,
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json))
);
CREATE TABLE components (
    id INTEGER PRIMARY KEY,
    role TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    path TEXT NOT NULL,
    sha256 TEXT NOT NULL CHECK(length(sha256) = 64),
    artifact_id INTEGER REFERENCES artifacts(id),
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    UNIQUE(role, name, version, sha256)
);
CREATE TABLE runs (
    id INTEGER PRIMARY KEY,
    environment_id INTEGER NOT NULL REFERENCES environments(id),
    phase TEXT NOT NULL CHECK(phase IN ('cold', 'warm')),
    repetition INTEGER NOT NULL,
    phase_definition TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    status TEXT NOT NULL,
    command_json TEXT NOT NULL CHECK(json_valid(command_json)),
    config_json TEXT NOT NULL CHECK(json_valid(config_json))
);
CREATE TABLE run_components (
    run_id INTEGER NOT NULL REFERENCES runs(id),
    component_id INTEGER NOT NULL REFERENCES components(id),
    PRIMARY KEY(run_id, component_id)
);
CREATE TABLE attempts (
    id INTEGER PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES runs(id),
    fixture_id INTEGER NOT NULL REFERENCES fixtures(id),
    started_at TEXT NOT NULL,
    elapsed_seconds REAL NOT NULL CHECK(elapsed_seconds >= 0),
    exit_code INTEGER,
    outcome TEXT NOT NULL,
    expected_outcome_met INTEGER NOT NULL CHECK(expected_outcome_met IN (0, 1)),
    command_json TEXT NOT NULL CHECK(json_valid(command_json)),
    stdout_artifact_id INTEGER NOT NULL REFERENCES artifacts(id),
    stderr_artifact_id INTEGER NOT NULL REFERENCES artifacts(id),
    UNIQUE(run_id, fixture_id)
);
CREATE TABLE events (
    attempt_id INTEGER NOT NULL REFERENCES attempts(id),
    ordinal INTEGER NOT NULL,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    PRIMARY KEY(attempt_id, ordinal)
);
CREATE TABLE page_outputs (
    id INTEGER PRIMARY KEY,
    attempt_id INTEGER NOT NULL REFERENCES attempts(id),
    fixture_page_id INTEGER REFERENCES fixture_pages(id),
    page_number INTEGER NOT NULL,
    status TEXT NOT NULL,
    raw_text TEXT NOT NULL,
    tsv_artifact_id INTEGER REFERENCES artifacts(id),
    raster_json TEXT NOT NULL CHECK(json_valid(raster_json)),
    timing_json TEXT NOT NULL CHECK(json_valid(timing_json)),
    UNIQUE(attempt_id, page_number)
);
CREATE TABLE ocr_regions (
    id INTEGER PRIMARY KEY,
    page_output_id INTEGER NOT NULL REFERENCES page_outputs(id),
    ordinal INTEGER NOT NULL,
    text TEXT NOT NULL,
    confidence REAL,
    x REAL NOT NULL, y REAL NOT NULL, width REAL NOT NULL, height REAL NOT NULL,
    coordinate_space TEXT NOT NULL,
    provenance_json TEXT NOT NULL CHECK(json_valid(provenance_json)),
    UNIQUE(page_output_id, ordinal)
);
CREATE TABLE page_artifacts (
    page_output_id INTEGER NOT NULL REFERENCES page_outputs(id),
    role TEXT NOT NULL,
    artifact_id INTEGER NOT NULL REFERENCES artifacts(id),
    PRIMARY KEY(page_output_id, role)
);
CREATE TABLE accuracy (
    page_output_id INTEGER PRIMARY KEY REFERENCES page_outputs(id),
    normalization TEXT NOT NULL,
    reference_characters INTEGER NOT NULL,
    character_edits INTEGER NOT NULL,
    cer REAL NOT NULL,
    reference_words INTEGER NOT NULL,
    word_edits INTEGER NOT NULL,
    wer REAL NOT NULL,
    omitted_words INTEGER NOT NULL,
    extra_words INTEGER NOT NULL,
    missing_words_json TEXT NOT NULL CHECK(json_valid(missing_words_json))
);
CREATE TABLE warnings (
    id INTEGER PRIMARY KEY,
    attempt_id INTEGER NOT NULL REFERENCES attempts(id),
    page_number INTEGER,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json))
);
CREATE TABLE errors (
    id INTEGER PRIMARY KEY,
    attempt_id INTEGER NOT NULL REFERENCES attempts(id),
    page_number INTEGER,
    category TEXT NOT NULL,
    message TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json))
);
CREATE TABLE resource_measurements (
    id INTEGER PRIMARY KEY,
    attempt_id INTEGER NOT NULL REFERENCES attempts(id),
    page_number INTEGER,
    metric TEXT NOT NULL,
    value REAL NOT NULL,
    unit TEXT NOT NULL,
    method TEXT NOT NULL
);
CREATE INDEX page_outputs_fixture ON page_outputs(fixture_page_id);
CREATE INDEX attempts_fixture ON attempts(fixture_id);

CREATE VIEW page_scores AS
SELECT r.phase, r.repetition, f.name AS fixture, p.page_number, p.status,
       a.cer, a.wer, a.omitted_words, a.extra_words, p.timing_json
FROM accuracy a JOIN page_outputs p ON p.id = a.page_output_id
JOIN attempts t ON t.id = p.attempt_id JOIN fixtures f ON f.id = t.fixture_id
JOIN runs r ON r.id = t.run_id;
