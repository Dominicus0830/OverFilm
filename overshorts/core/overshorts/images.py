"""첨부 그림 검사 (기획서 3.2 이미지 처리, M1).

- 얼굴: OpenCV YuNet 으로 찾아 흐림 처리한 사본을 만든다. 원본은 화면에 쓰지 않는다.
- 선정성: ViT 분류 모델(ONNX)에 걸리면 그 그림은 뺀다.
- 검사를 못 한 그림(모델 없음, 그림을 못 읽음)은 사람이 보도록 `review` 로 남긴다.

GPU: onnxruntime 의 DirectML(라데온·NVIDIA·인텔 모두) → CUDA → CPU 순으로 고른다.
모델은 이 단계 동안만 올리고, 같은 단계의 그림을 모아서 한 번에 처리한다 (VRAM 스케줄러 원칙).
"""

from __future__ import annotations

import hashlib
import json
import os
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .db import Store

# 모델은 첫 사용 때 받아서 sha256 을 확인한다.
MODELS = {
    "yunet": {
        "file": "face_detection_yunet_2023mar.onnx",
        "url": "https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/"
        "face_detection_yunet/face_detection_yunet_2023mar.onnx",
        "sha256": "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
        "license": "MIT (OpenCV Zoo)",
    },
    "nsfw": {
        "file": "nsfw_image_detection_quantized.onnx",
        "url": "https://huggingface.co/onnx-community/nsfw_image_detection-ONNX/resolve/main/"
        "onnx/model_quantized.onnx",
        "sha256": "d9afb1e057104e6cc8616d174f0f6a8b8b0389c839eea7d5efa4bdf1a77efd27",
        "license": "Apache-2.0 (Falconsai/nsfw_image_detection)",
    },
}

MAX_DETECT_SIDE = 1280
MAX_PIXELS = 40_000_000  # 이보다 큰 그림은 읽지 않는다
NSFW_THRESHOLD = 0.7
FACE_SCORE = 0.7


class ModelError(Exception):
    pass


def ensure_model(name: str, models_dir: Path, download: bool = True) -> Path:
    spec = MODELS.get(name)
    if spec is None:
        raise ModelError(f"알 수 없는 모델: {name}")
    path = models_dir / spec["file"]
    if path.exists() and _sha256(path) == spec["sha256"]:
        return path
    if not download:
        raise ModelError(f"모델이 없습니다: {path}")
    models_dir.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".part")
    try:
        with urllib.request.urlopen(spec["url"], timeout=120) as r, open(tmp, "wb") as f:
            total = 0
            while chunk := r.read(1 << 20):
                total += len(chunk)
                if total > 500 * 1024 * 1024:
                    raise ModelError("모델 파일이 예상보다 큽니다")
                f.write(chunk)
    except (OSError, ValueError) as err:
        raise ModelError(f"모델을 받지 못했습니다 ({name}): {err}") from err
    if _sha256(tmp) != spec["sha256"]:
        tmp.unlink(missing_ok=True)
        raise ModelError(f"모델 파일 확인값이 다릅니다 ({name})")
    os.replace(tmp, path)
    return path


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def pick_providers() -> list[str]:
    """라데온도 지원 (사용자 결정 2026-11-14): DirectML 이 있으면 먼저 쓴다."""
    try:
        import onnxruntime as ort
    except ImportError:
        return []
    have = set(ort.get_available_providers())
    order = ["DmlExecutionProvider", "ROCMExecutionProvider", "CUDAExecutionProvider",
             "CPUExecutionProvider"]
    return [p for p in order if p in have]


@dataclass
class Verdict:
    faces: int | None  # None = 검사 못 함
    nsfw: float | None  # 선정성 확률, None = 검사 못 함
    blurred: str | None  # 흐림 처리한 사본 경로 (얼굴이 있을 때)
    dropped: bool
    note: str = ""


class ImageInspector:
    def __init__(self, models_dir: str | Path, download: bool = True,
                 blur_faces: bool = True, drop_nsfw: bool = True):
        self.models_dir = Path(models_dir)
        self.download = download
        self.blur_faces = blur_faces
        self.drop_nsfw = drop_nsfw
        self._face = None
        self._nsfw = None
        self.provider: str | None = None
        self.load_errors: list[str] = []

    def load(self) -> None:
        try:
            import cv2  # noqa: F401
            import numpy  # noqa: F401
        except ImportError:
            self.load_errors.append("opencv-python-headless·numpy 가 없어 그림 검사를 못 합니다")
            return
        import cv2

        if self.blur_faces:
            try:
                path = ensure_model("yunet", self.models_dir, self.download)
                self._face = cv2.FaceDetectorYN.create(str(path), "", (320, 320), FACE_SCORE)
            except (ModelError, cv2.error) as err:
                self.load_errors.append(f"얼굴 검출 모델: {err}")
        if self.drop_nsfw:
            providers = pick_providers()
            if not providers:
                self.load_errors.append("onnxruntime 이 없어 선정성 검사를 못 합니다")
                return
            try:
                import onnxruntime as ort

                path = ensure_model("nsfw", self.models_dir, self.download)
                self._nsfw = ort.InferenceSession(str(path), providers=providers)
                self.provider = self._nsfw.get_providers()[0]
            except ModelError as err:
                self.load_errors.append(f"선정성 모델: {err}")
            except Exception as err:  # noqa: BLE001 - onnxruntime 오류 종류가 많다
                self.load_errors.append(f"선정성 모델을 올리지 못했습니다: {err}")

    def unload(self) -> None:
        """다음 단계 모델이 VRAM 을 쓰도록 내린다."""
        self._face = None
        self._nsfw = None

    def inspect(self, data: bytes, out_path: Path) -> Verdict:
        import cv2
        import numpy as np

        arr = np.frombuffer(data, dtype=np.uint8)
        img = cv2.imdecode(arr, cv2.IMREAD_COLOR)
        if img is None:
            return Verdict(None, None, None, False, "그림을 읽지 못했습니다 (움짤·손상)")
        h, w = img.shape[:2]
        if h <= 0 or w <= 0 or h * w > MAX_PIXELS:
            return Verdict(None, None, None, False, "그림 크기가 비정상입니다")

        nsfw = self._nsfw_score(img) if self._nsfw is not None else None
        if nsfw is not None and nsfw >= NSFW_THRESHOLD:
            return Verdict(None, nsfw, None, True, "선정성 분류에 걸려 뺐습니다")

        faces = None
        blurred = None
        if self._face is not None:
            boxes = self._detect(img)
            faces = len(boxes)
            if boxes:
                for x, y, bw, bh in boxes:
                    _blur(img, x, y, bw, bh)
                out_path.parent.mkdir(parents=True, exist_ok=True)
                if cv2.imwrite(str(out_path), img):
                    blurred = str(out_path)
        note = "; ".join(self.load_errors)
        return Verdict(faces, nsfw, blurred, False, note)

    def _detect(self, img) -> list[tuple[int, int, int, int]]:
        import cv2

        h, w = img.shape[:2]
        scale = min(1.0, MAX_DETECT_SIDE / max(h, w))
        small = cv2.resize(img, (max(1, int(w * scale)), max(1, int(h * scale)))) if scale < 1 else img
        sh, sw = small.shape[:2]
        self._face.setInputSize((sw, sh))
        _, found = self._face.detect(small)
        boxes = []
        if found is None:
            return boxes
        for row in found:
            x, y, bw, bh = (float(v) / scale for v in row[:4])
            boxes.append((int(x), int(y), int(bw), int(bh)))
        return boxes

    def _nsfw_score(self, img) -> float | None:
        import cv2
        import numpy as np

        rgb = cv2.cvtColor(cv2.resize(img, (224, 224), interpolation=cv2.INTER_LINEAR),
                           cv2.COLOR_BGR2RGB)
        x = (rgb.astype(np.float32) / 255.0 - 0.5) / 0.5
        x = np.transpose(x, (2, 0, 1))[None, ...]
        try:
            name = self._nsfw.get_inputs()[0].name
            logits = self._nsfw.run(None, {name: x})[0][0]
        except Exception:  # noqa: BLE001
            return None
        e = np.exp(logits - np.max(logits))
        probs = e / e.sum()
        return float(probs[1]) if len(probs) > 1 else None


def _blur(img, x: int, y: int, w: int, h: int) -> None:
    import cv2

    H, W = img.shape[:2]
    pad_w, pad_h = int(w * 0.15), int(h * 0.15)
    x0, y0 = max(0, x - pad_w), max(0, y - pad_h)
    x1, y1 = min(W, x + w + pad_w), min(H, y + h + pad_h)
    if x1 <= x0 or y1 <= y0:
        return
    roi = img[y0:y1, x0:x1]
    k = max(15, (max(x1 - x0, y1 - y0) // 4) | 1)
    img[y0:y1, x0:x1] = cv2.GaussianBlur(roi, (k, k), 0)


def inspect_pending(store: Store, inspector: ImageInspector, image_dir: Path,
                    limit: int = 200) -> dict[str, int]:
    """그림을 받아 두었지만 아직 검사 안 한 글을 한 번에 검사한다."""
    rows = store.conn.execute(
        "SELECT id, gall_id, post_no, images FROM posts WHERE images LIKE '%\"inspected\": false%' "
        "ORDER BY id LIMIT ?",
        (max(1, min(limit, 10_000)),),
    ).fetchall()
    counts = {"posts": 0, "images": 0, "faces": 0, "dropped": 0, "unchecked": 0}
    if not rows:
        return counts
    inspector.load()
    try:
        for row in rows:
            images: list[dict[str, Any]] = json.loads(row["images"] or "[]")
            needs_review: list[str] = []
            for i, item in enumerate(images):
                if item.get("kind", "image") != "image":
                    continue  # 동영상 프레임 검사는 아직 (원문 미디어 분석과 함께)
                if item.get("inspected") is not False or not item.get("local"):
                    continue
                try:
                    data = Path(item["local"]).read_bytes()
                except OSError:
                    item.update(inspected=True, note="받은 그림 파일이 없습니다")
                    needs_review.append(item["note"])
                    continue
                out = image_dir / row["gall_id"] / f"{row['post_no']}-{i}-blur.jpg"
                v = inspector.inspect(data, out)
                item.update(inspected=True, faces=v.faces, nsfw=v.nsfw, blurred=v.blurred,
                            dropped=v.dropped, note=v.note)
                counts["images"] += 1
                counts["faces"] += 1 if v.faces else 0
                counts["dropped"] += 1 if v.dropped else 0
                missed = (inspector.blur_faces and v.faces is None and not v.dropped) or (
                    inspector.drop_nsfw and v.nsfw is None)
                if missed:
                    counts["unchecked"] += 1
                    needs_review.append(v.note or "검사 못 한 그림")
            store.conn.execute("UPDATE posts SET images=? WHERE id=?",
                               (json.dumps(images, ensure_ascii=False), row["id"]))
            store.conn.commit()
            verdict = "review" if needs_review else "pass"
            store.add_filter_result(row["id"], "image", verdict, sorted(set(needs_review)),
                                    model=inspector.provider)
            counts["posts"] += 1
    finally:
        inspector.unload()
    return counts
