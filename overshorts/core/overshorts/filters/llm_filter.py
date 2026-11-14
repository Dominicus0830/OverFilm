"""LLM 필터 (3.2 의 2차). 로컬 GPU LLM(Ollama)이 맡아 Claude 사용량을 아낀다.

규칙 필터를 통과한 글(jobs.status = crawled)만 돈다. 같은 단계의 글을 모아서 처리하고
끝나면 모델을 내린다 (VRAM 스케줄러).
"""

from __future__ import annotations

import json

from ..db import Store
from ..llm import LimitReached, LlmError, Router
from ..script.models import CATEGORY_KO, FilterAnswer
from ..script.prompts import filter_task
from ..script.segments import comment_label


def decide(ans: FilterAnswer, block_categories: list[str]) -> tuple[str, list[str]]:
    """모델 대답을 설정에 맞춰 최종 판정으로. 막을 범주에 걸리면 block."""
    reasons = list(ans.reasons)
    hit = [c for c in ans.categories if c in block_categories]
    verdict = ans.verdict
    if hit:
        verdict = "block"
        reasons.insert(0, "막는 범주: " + ", ".join(CATEGORY_KO.get(c, c) for c in hit))
    elif verdict == "block" and ans.categories:
        # 모델이 막았지만 사용자가 그 범주를 허용한 경우 → 사람이 보게
        verdict = "review"
    return verdict, reasons


def filter_pending(store: Store, router: Router, block_categories: list[str],
                   allow_topics: list[str], limit: int = 50) -> dict[str, int]:
    rows = store.conn.execute(
        "SELECT j.id AS job_id, p.id AS post_id, p.title, p.body, p.comments FROM jobs j "
        "JOIN posts p ON p.id = j.post_id WHERE j.status = 'crawled' AND p.discarded = 0 "
        "ORDER BY j.id LIMIT ?",
        (max(1, min(limit, 1000)),),
    ).fetchall()
    counts = {"pass": 0, "review": 0, "block": 0, "failed": 0}
    for row in rows:
        comments = [comment_label(c) for c in json.loads(row["comments"] or "[]")
                    if isinstance(c, dict)]
        try:
            ans, model = router.ask(
                filter_task(row["title"], row["body"], comments, block_categories, allow_topics),
                FilterAnswer)
        except LimitReached:
            counts["failed"] += 1
            break
        except LlmError as err:
            store.set_job_status(row["job_id"], "crawled", f"LLM 필터: {err}")
            counts["failed"] += 1
            continue
        verdict, reasons = decide(ans, block_categories)
        store.add_filter_result(row["post_id"], "llm", verdict, reasons, ans.fit_score, model)
        # 파이프라인 도식: pass 만 대본으로 넘어간다. review 는 멈춰 두고 사람이 패널에서
        # 통과시키면(approve_review) 대본 단계로 간다.
        if verdict == "pass":
            store.set_job_status(row["job_id"], "filtered", None)
        else:
            prefix = "검토 필요: " if verdict == "review" else ""
            store.set_job_status(row["job_id"], "blocked", prefix + "; ".join(reasons))
        counts[verdict] += 1
    return counts


def approve_review(store: Store, post_id: int) -> bool:
    """LLM 필터가 review 로 멈춘 글을 사람이 통과시킨다 → 대본 단계로."""
    cur = store.conn.execute(
        "UPDATE jobs SET status='filtered', error=NULL WHERE post_id=? AND status='blocked' "
        "AND error LIKE '검토 필요:%'", (post_id,))
    store.conn.commit()
    if cur.rowcount:
        store.add_filter_result(post_id, "human", "pass", ["사람이 통과시킴"])
    return cur.rowcount > 0
