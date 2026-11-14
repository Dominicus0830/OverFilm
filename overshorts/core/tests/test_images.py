import json
import os
from pathlib import Path

import pytest

from overshorts.db import Store
from overshorts.images import ImageInspector, inspect_pending

cv2 = pytest.importorskip("cv2")
np = pytest.importorskip("numpy")

# 모델이 있는 폴더를 OVERSHORTS_MODELS 로 주면 실제 모델로도 돈다 (없으면 그 테스트만 건너뜀)
MODELS = os.environ.get("OVERSHORTS_MODELS")


def _jpeg(color=(200, 180, 160)) -> bytes:
    img = np.full((240, 320, 3), color, dtype=np.uint8)
    ok, buf = cv2.imencode(".jpg", img)
    assert ok
    return buf.tobytes()


def _store_with_image(tmp_path: Path, data: bytes) -> Store:
    store = Store(":memory:")
    local = tmp_path / "a.img"
    local.write_bytes(data)
    store.upsert_post({"gall_id": "g", "post_no": 1, "title": "t", "body": "b",
                       "images": [{"url": "u", "local": str(local), "inspected": False}]})
    return store


def test_without_models_marks_review(tmp_path):
    store = _store_with_image(tmp_path, _jpeg())
    insp = ImageInspector(tmp_path / "none", download=False)
    counts = inspect_pending(store, insp, tmp_path / "out")
    assert counts == {"posts": 1, "images": 1, "faces": 0, "dropped": 0, "unchecked": 1}
    row = store.conn.execute("SELECT verdict FROM filter_results WHERE stage='image'").fetchone()
    assert row["verdict"] == "review"
    item = json.loads(store.conn.execute("SELECT images FROM posts").fetchone()["images"])[0]
    assert item["inspected"] is True
    # 두 번 돌려도 다시 검사하지 않는다
    assert inspect_pending(store, insp, tmp_path / "out")["posts"] == 0


def test_broken_image_is_not_a_crash(tmp_path):
    store = _store_with_image(tmp_path, b"\x89PNG\r\n\x1a\nbroken")
    counts = inspect_pending(store, ImageInspector(tmp_path, download=False), tmp_path / "out")
    assert counts["unchecked"] == 1


def test_checks_off_means_pass(tmp_path):
    store = _store_with_image(tmp_path, _jpeg())
    insp = ImageInspector(tmp_path, download=False, blur_faces=False, drop_nsfw=False)
    assert inspect_pending(store, insp, tmp_path / "out")["unchecked"] == 0


@pytest.mark.skipif(not MODELS, reason="OVERSHORTS_MODELS 없음")
def test_real_models_plain_picture(tmp_path):
    insp = ImageInspector(MODELS, download=False)
    insp.load()
    assert insp.load_errors == []
    v = insp.inspect(_jpeg(), tmp_path / "x.jpg")
    assert v.faces == 0 and v.nsfw is not None and v.nsfw < 0.7 and not v.dropped
