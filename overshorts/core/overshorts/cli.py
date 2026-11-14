"""`overshorts` 명령 (MVP 동안 패널 대신 쓰는 입구).

    overshorts parse <링크>          링크 종류만 판단 (요청 안 함)
    overshorts ingest <링크> [옵션]   수집 + 1차 규칙 필터
    overshorts inspect                받은 그림 얼굴 흐림·선정성 검사 (모아서 한 번에)
    overshorts filter                 LLM 필터 (로컬 GPU LLM)
    overshorts script [--post N]      대본 쓰기 + 규칙·오글 검사
    overshorts budget                 오늘 요청 예산·쿨다운 상태
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .config import crawl_limits, load_config, rule_config
from .crawler.budget import CrawlGuard
from .crawler.collector import Collector, GalleryOptions
from .crawler.fetcher import Fetcher
from .db import Store
from .urls import UrlError, parse_url


def _store(cfg: dict) -> tuple[Store, Path]:
    data = Path(str(cfg.get("data_dir") or "./data"))
    return Store(data / "overshorts.db"), data


def build_router(cfg: dict):
    from .llm import Router
    from .llm.claude_code import ClaudeCodeClient
    from .llm.ollama import OllamaClient

    llm = cfg.get("llm") or {}
    ollama = OllamaClient(model=str(llm.get("ollama_model") or "qwen3:8b"),
                          host=str(llm.get("ollama_host") or "http://127.0.0.1:11434"))
    claude = ClaudeCodeClient(model=str(llm.get("claude_model") or ""))

    def pick(name: str):
        return claude if name == "claude_code" else ollama

    clients = {"filter": pick(str(llm.get("filter") or "ollama")),
               "script": pick(str(llm.get("script") or "claude_code"))}
    clients["cringe"] = clients["script"]
    fallback = pick(str(llm.get("fallback") or "ollama"))
    on_limit = "wait" if llm.get("on_limit") == "wait" else "fallback"
    return Router(clients, fallback, on_limit,
                  on_event=lambda m: print(f"경고: {m}", file=sys.stderr)), ollama


def _llm_stage(args, cfg: dict, store: Store) -> int:
    router, ollama = build_router(cfg)
    if args.cmd == "filter":
        from .filters.llm_filter import filter_pending

        f = cfg.get("filter") or {}
        try:
            counts = filter_pending(store, router, list(f.get("block_categories") or []),
                                    [str(t) for t in f.get("allow_topics") or []])
        finally:
            ollama.unload()
        print(f"통과 {counts['pass']}, 검수 {counts['review']}, 막힘 {counts['block']}, "
              f"실패 {counts['failed']}")
        return 0
    from .script.writer import ScriptSettings, script_pending

    render = cfg.get("render") or {}
    style = cfg.get("style") or {}
    settings = ScriptSettings(
        style_dir=style.get("dir") or "./style",
        tone=args.tone or str(render.get("tone") or "담백"),
        speech=str(style.get("speech") or "반말"),
        min_cringe_score=float(style.get("min_cringe_score") or 7),
    )
    try:
        counts = script_pending(store, router, settings, args.post)
    finally:
        ollama.unload()
    print(f"대본 {counts['scripted']}개 (경고 붙음 {counts['warned']}), 실패 {counts['failed']}")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="overshorts", description="DC 갤러리 → 쇼츠 자동화")
    ap.add_argument("--config", default="config.yaml", help="설정 파일 (없으면 기본값)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("parse", help="링크 종류 판단")
    p.add_argument("url")

    i = sub.add_parser("ingest", help="수집 + 규칙 필터")
    i.add_argument("url")
    i.add_argument("--recommend-only", action="store_true", help="개념글만")
    i.add_argument("--min-upvotes", type=int, default=0)
    i.add_argument("--within-hours", type=float, default=24.0)
    i.add_argument("--max-posts", type=int, default=10)

    sub.add_parser("budget", help="요청 예산·쿨다운 상태")
    sub.add_parser("filter", help="규칙 통과 글을 LLM 필터로 분류 (로컬 Ollama)")
    sc = sub.add_parser("script", help="필터 통과 글의 대본 쓰기 (Claude Code)")
    sc.add_argument("--post", type=int, action="append", help="이 글만 (여러 번 가능)")
    sc.add_argument("--tone", choices=["담백", "드립 조금", "정리형"])
    ins = sub.add_parser("inspect", help="받은 그림의 얼굴 흐림·선정성 검사")
    ins.add_argument("--no-download", action="store_true", help="모델을 받지 않음 (없으면 검사 못 함)")

    args = ap.parse_args(argv)
    cfg = load_config(args.config)

    if args.cmd == "parse":
        try:
            u = parse_url(args.url)
        except UrlError as err:
            print(f"알아볼 수 없는 링크: {err}", file=sys.stderr)
            return 2
        print(json.dumps({**u.__dict__, "label": u.label}, ensure_ascii=False))
        return 0

    store, data = _store(cfg)
    try:
        limits = crawl_limits(cfg)
        guard = CrawlGuard(store, limits)
        if args.cmd == "inspect":
            from .images import ImageInspector, inspect_pending

            f = cfg.get("filter") or {}
            inspector = ImageInspector(data / "models", download=not args.no_download,
                                       blur_faces=bool(f.get("blur_faces", True)),
                                       drop_nsfw=bool(f.get("drop_nsfw_images", True)))
            counts = inspect_pending(store, inspector, data / "images")
            print(f"글 {counts['posts']}개, 그림 {counts['images']}장 검사: 얼굴 흐림 "
                  f"{counts['faces']}장, 선정성으로 뺌 {counts['dropped']}장, "
                  f"검사 못 함 {counts['unchecked']}장 (장치: {inspector.provider or '없음'})")
            for e in inspector.load_errors:
                print(f"  경고: {e}", file=sys.stderr)
            return 0
        if args.cmd in ("filter", "script"):
            return _llm_stage(args, cfg, store)
        if args.cmd == "budget":
            print(json.dumps(guard.status(), ensure_ascii=False))
            for w in limits.warnings():
                print(f"경고: {w}", file=sys.stderr)
            return 0
        try:
            parse_url(args.url)
        except UrlError as err:
            print(f"알아볼 수 없는 링크: {err}", file=sys.stderr)
            return 2
        for w in limits.warnings():
            print(f"경고: {w}", file=sys.stderr)
        crawler = cfg.get("crawler") or {}
        fetcher = Fetcher(guard, proxy=crawler.get("proxy"))
        collector = Collector(
            store, fetcher, rule_config(cfg), cache_dir=data / "cache",
            max_comments=int(crawler.get("max_comments") or 50),
            image_dir=data / "images", max_images=int(crawler.get("max_images") or 5),
        )
        opts = GalleryOptions(
            recommend_only=args.recommend_only,
            min_upvotes=max(0, args.min_upvotes),
            within_hours=args.within_hours if args.within_hours > 0 else None,
            max_posts=max(1, min(args.max_posts, 200)),
        )
        report = collector.ingest(args.url, opts)
        print(report.summary())
        for no, reasons in report.blocked:
            print(f"  막힘 {no}: {'; '.join(reasons)}")
        for err in report.errors:
            print(f"  오류: {err}")
        return 1 if report.stopped and not report.saved else 0
    finally:
        store.close()


if __name__ == "__main__":
    raise SystemExit(main())
