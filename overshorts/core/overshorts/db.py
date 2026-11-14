"""SQLite 저장소 (기획서 6.1).

단계마다 결과를 여기와 작업 폴더에 남겨서, 실패하면 그 단계부터 다시 돈다.
크롤러 요청 기록(`requests`)과 쿨다운 상태(`crawler_state`)도 여기 둬서
프로그램을 껐다 켜도 예산과 쿨다운이 이어진다.
"""

from __future__ import annotations

import json
import sqlite3
from datetime import datetime
from pathlib import Path
from typing import Any

SCHEMA = """
CREATE TABLE IF NOT EXISTS sources (
    id INTEGER PRIMARY KEY,
    url TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    gall_type TEXT NOT NULL,
    gall_id TEXT NOT NULL,
    options TEXT NOT NULL DEFAULT '{}',
    enabled INTEGER NOT NULL DEFAULT 1,
    last_checked TEXT
);
CREATE TABLE IF NOT EXISTS posts (
    id INTEGER PRIMARY KEY,
    gall_id TEXT NOT NULL,
    post_no INTEGER NOT NULL,
    gall_type TEXT NOT NULL DEFAULT 'board',
    gall_name TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT '',
    images TEXT NOT NULL DEFAULT '[]',
    comments TEXT NOT NULL DEFAULT '[]',
    upvotes INTEGER NOT NULL DEFAULT 0,
    posted_at TEXT,
    content_hash TEXT NOT NULL DEFAULT '',
    discarded INTEGER NOT NULL DEFAULT 0,
    fetched_at TEXT NOT NULL,
    UNIQUE(gall_id, post_no)
);
CREATE TABLE IF NOT EXISTS filter_results (
    id INTEGER PRIMARY KEY,
    post_id INTEGER NOT NULL REFERENCES posts(id),
    stage TEXT NOT NULL,
    verdict TEXT NOT NULL,
    reasons TEXT NOT NULL DEFAULT '[]',
    fit_score REAL,
    model TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS scripts (
    id INTEGER PRIMARY KEY,
    post_id INTEGER NOT NULL REFERENCES posts(id),
    version INTEGER NOT NULL,
    tone TEXT NOT NULL,
    lines TEXT NOT NULL DEFAULT '[]',
    overlap_ratio REAL,
    model TEXT
);
CREATE TABLE IF NOT EXISTS voices (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    engine TEXT NOT NULL,
    ref_wav TEXT,
    ref_text TEXT,
    consent INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS jobs (
    id INTEGER PRIMARY KEY,
    post_id INTEGER NOT NULL REFERENCES posts(id),
    script_id INTEGER,
    voice_id INTEGER,
    preset TEXT NOT NULL DEFAULT 'card',
    status TEXT NOT NULL DEFAULT 'queued',
    error TEXT,
    spec_path TEXT,
    mp4_path TEXT,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS requests (
    id INTEGER PRIMARY KEY,
    ts REAL NOT NULL,
    url TEXT NOT NULL,
    status INTEGER,
    elapsed REAL,
    outcome TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS requests_ts ON requests(ts);
CREATE TABLE IF NOT EXISTS crawler_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"""

JOB_STATUSES = (
    "queued",
    "crawled",
    "filtered",
    "scripted",
    "voiced",
    "aligned",
    "preview_ready",
    "approved",
    "rendered",
    "blocked",
    "rejected",
    "failed",
)


def now_iso() -> str:
    return datetime.now().isoformat(timespec="seconds")


class Store:
    def __init__(self, path: str | Path):
        self.path = str(path)
        if self.path != ":memory:":
            Path(self.path).parent.mkdir(parents=True, exist_ok=True)
        self.conn = sqlite3.connect(self.path, timeout=30)
        self.conn.row_factory = sqlite3.Row
        self.conn.execute("PRAGMA foreign_keys = ON")
        if self.path != ":memory:":
            # 패널(여러 요청)과 작업 스레드가 같은 DB 를 함께 쓴다
            self.conn.execute("PRAGMA journal_mode = WAL")
        self.conn.executescript(SCHEMA)
        self._migrate()
        self.conn.commit()

    def _migrate(self) -> None:
        """예전 DB 에 새 열을 더한다."""
        cols = {r["name"] for r in self.conn.execute("PRAGMA table_info(posts)").fetchall()}
        if "gall_name" not in cols:
            self.conn.execute("ALTER TABLE posts ADD COLUMN gall_name TEXT NOT NULL DEFAULT ''")

    def close(self) -> None:
        self.conn.close()

    # --- 크롤러 상태 ---
    def get_state(self, key: str, default: str | None = None) -> str | None:
        row = self.conn.execute("SELECT value FROM crawler_state WHERE key=?", (key,)).fetchone()
        return row["value"] if row else default

    def set_state(self, key: str, value: str) -> None:
        self.conn.execute(
            "INSERT INTO crawler_state(key, value) VALUES(?, ?) "
            "ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            (key, value),
        )
        self.conn.commit()

    def log_request(
        self, ts: float, url: str, status: int | None, elapsed: float | None, outcome: str
    ) -> None:
        self.conn.execute(
            "INSERT INTO requests(ts, url, status, elapsed, outcome) VALUES(?,?,?,?,?)",
            (ts, url[:500], status, elapsed, outcome),
        )
        self.conn.commit()

    def count_requests_since(self, ts: float) -> int:
        row = self.conn.execute("SELECT COUNT(*) AS n FROM requests WHERE ts >= ?", (ts,)).fetchone()
        return int(row["n"]) if row else 0

    def recent_elapsed(self, limit: int = 20) -> list[float]:
        rows = self.conn.execute(
            "SELECT elapsed FROM requests WHERE outcome='ok' AND elapsed IS NOT NULL "
            "ORDER BY ts DESC LIMIT ?",
            (limit,),
        ).fetchall()
        return [float(r["elapsed"]) for r in rows]

    # --- 글 ---
    def has_post(self, gall_id: str, post_no: int) -> bool:
        row = self.conn.execute(
            "SELECT 1 FROM posts WHERE gall_id=? AND post_no=?", (gall_id, post_no)
        ).fetchone()
        return row is not None

    def upsert_post(self, record: dict[str, Any]) -> int:
        fields = {
            "gall_id": record["gall_id"],
            "post_no": int(record["post_no"]),
            "gall_type": record.get("gall_type", "board"),
            "gall_name": record.get("gall_name", ""),
            "title": record.get("title", ""),
            "subject": record.get("subject", ""),
            "body": record.get("body", ""),
            "images": json.dumps(record.get("images", []), ensure_ascii=False),
            "comments": json.dumps(record.get("comments", []), ensure_ascii=False),
            "upvotes": int(record.get("upvotes", 0)),
            "posted_at": record.get("posted_at"),
            "content_hash": record.get("content_hash", ""),
            "fetched_at": now_iso(),
        }
        cols = ", ".join(fields)
        marks = ", ".join("?" for _ in fields)
        updates = ", ".join(f"{k}=excluded.{k}" for k in fields if k not in ("gall_id", "post_no"))
        self.conn.execute(
            f"INSERT INTO posts({cols}) VALUES({marks}) "
            f"ON CONFLICT(gall_id, post_no) DO UPDATE SET {updates}",
            tuple(fields.values()),
        )
        self.conn.commit()
        row = self.conn.execute(
            "SELECT id FROM posts WHERE gall_id=? AND post_no=?",
            (fields["gall_id"], fields["post_no"]),
        ).fetchone()
        return int(row["id"])

    def is_discarded(self, gall_id: str, post_no: int) -> bool:
        row = self.conn.execute(
            "SELECT discarded FROM posts WHERE gall_id=? AND post_no=?", (gall_id, post_no)
        ).fetchone()
        return bool(row and row["discarded"])

    def discard_post(self, post_id: int) -> None:
        """검수의 '폐기': 같은 글을 다시 수집하지 않는다."""
        self.conn.execute("UPDATE posts SET discarded=1 WHERE id=?", (post_id,))
        self.conn.commit()

    def add_filter_result(
        self,
        post_id: int,
        stage: str,
        verdict: str,
        reasons: list[str],
        fit_score: float | None = None,
        model: str | None = None,
    ) -> None:
        self.conn.execute(
            "INSERT INTO filter_results(post_id, stage, verdict, reasons, fit_score, model, "
            "created_at) VALUES(?,?,?,?,?,?,?)",
            (post_id, stage, verdict, json.dumps(reasons, ensure_ascii=False), fit_score, model,
             now_iso()),
        )
        self.conn.commit()

    def add_job(self, post_id: int, status: str, error: str | None = None) -> int:
        if status not in JOB_STATUSES:
            raise ValueError(f"알 수 없는 작업 상태: {status}")
        cur = self.conn.execute(
            "INSERT INTO jobs(post_id, status, error, preset, updated_at) VALUES(?,?,?,'auto',?)",
            (post_id, status, error, now_iso()),
        )
        self.conn.commit()
        return int(cur.lastrowid or 0)

    def set_job_status(self, job_id: int, status: str, error: str | None = None,
                       script_id: int | None = None) -> None:
        if status not in JOB_STATUSES:
            raise ValueError(f"알 수 없는 작업 상태: {status}")
        if script_id is None:
            self.conn.execute("UPDATE jobs SET status=?, error=?, updated_at=? WHERE id=?",
                              (status, error, now_iso(), job_id))
        else:
            self.conn.execute(
                "UPDATE jobs SET status=?, error=?, script_id=?, updated_at=? WHERE id=?",
                (status, error, script_id, now_iso(), job_id))
        self.conn.commit()

    def next_script_version(self, post_id: int) -> int:
        row = self.conn.execute("SELECT MAX(version) AS v FROM scripts WHERE post_id=?",
                                (post_id,)).fetchone()
        return int(row["v"] or 0) + 1

    def add_script(self, post_id: int, version: int, tone: str, payload: dict[str, Any],
                   overlap_ratio: float, model: str) -> int:
        cur = self.conn.execute(
            "INSERT INTO scripts(post_id, version, tone, lines, overlap_ratio, model) "
            "VALUES(?,?,?,?,?,?)",
            (post_id, version, tone, json.dumps(payload, ensure_ascii=False), overlap_ratio,
             model))
        self.conn.commit()
        return int(cur.lastrowid or 0)

    def upsert_source(self, url: str, kind: str, gall_type: str, gall_id: str,
                      options: dict[str, Any]) -> None:
        self.conn.execute(
            "INSERT INTO sources(url, kind, gall_type, gall_id, options, last_checked) "
            "VALUES(?,?,?,?,?,?) ON CONFLICT(url) DO UPDATE SET options=excluded.options, "
            "gall_type=excluded.gall_type, last_checked=excluded.last_checked",
            (url, kind, gall_type, gall_id, json.dumps(options, ensure_ascii=False), now_iso()),
        )
        self.conn.commit()
