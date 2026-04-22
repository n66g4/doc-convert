"""
docling_adapter/plugin_main.py
薄适配层：将 JSON-RPC ctx 映射为 docling 文档管线调用。
若导出 Markdown 含默认占位符 `<!-- image -->`，则对对应 PictureItem 做 RapidOCR 并插入识别文字。
"""
from __future__ import annotations

import json
import logging
import os
import re
import sys
from pathlib import Path
from typing import Any

# Docling 与 Markdown 序列化默认一致（见 docling_core PictureItem.export_to_markdown）
DEFAULT_IMAGE_PLACEHOLDER = "<!-- image -->"


def _plugin_logger() -> logging.Logger:
    """日志写到 stderr，由 Core 的 Python worker 汇总进 docconvert.log（成功时也会记录 stderr）。"""
    name = "docling_adapter"
    log = logging.getLogger(name)
    if log.handlers:
        return log
    log.setLevel(logging.DEBUG)
    h = logging.StreamHandler(sys.stderr)
    h.setFormatter(logging.Formatter("%(name)s: %(levelname)s: %(message)s"))
    log.addHandler(h)
    log.propagate = False
    return log


def _flush_diagnostic_streams(log: logging.Logger) -> None:
    """确保诊断行在进程退出前进入管道，避免 Core 读到空 stderr。"""
    for h in log.handlers:
        try:
            h.flush()
        except Exception:
            pass
    try:
        sys.stderr.flush()
    except Exception:
        pass


def _rapidocr_params_for_docconvert() -> dict[str, Any] | None:
    """Core 通过 DOCCONVERT_RAPIDOCR_MODEL_DIR 指定可写模型根目录（如 Application Support），避免写入 .app 内 site-packages。"""
    import os

    root = (os.environ.get("DOCCONVERT_RAPIDOCR_MODEL_DIR") or "").strip()
    if not root:
        return None
    p = Path(root)
    try:
        p.mkdir(parents=True, exist_ok=True)
    except OSError:
        pass
    return {"Global.model_root_dir": str(p.resolve())}


def _quiet_rapidocr_logging() -> None:
    """RapidOCR 默认会向 stderr 刷大量 [INFO]，易挤占 Core 对 stderr 的截断预算，导致末尾槽位日志丢失。"""
    for name in list(logging.Logger.manager.loggerDict.keys()):
        if not isinstance(name, str) or "rapid" not in name.lower():
            continue
        obj = logging.Logger.manager.loggerDict[name]
        if isinstance(obj, logging.Logger):
            obj.setLevel(logging.WARNING)
    for name in ("rapidocr", "RapidOCR"):
        logging.getLogger(name).setLevel(logging.WARNING)


def health(ctx: dict | None = None) -> dict:
    """供 Core/UI 轻量自检：仅检测 docling 包是否可见，不 import DocumentConverter（避免拉起 torch 等重依赖）。"""
    import importlib.util

    if importlib.util.find_spec("docling") is None:
        return {"status": "error", "message": "未安装 docling 包"}
    return {
        "status": "ok",
        "message": "Docling 包已安装（完整链路请用深度自检）",
    }


def _options_flag(options: Any, key: str, default: bool) -> bool:
    if not isinstance(options, dict):
        return default
    v = options.get(key, default)
    if isinstance(v, bool):
        return v
    return default


def _options_float(options: Any, key: str, default: float) -> float:
    if not isinstance(options, dict):
        return default
    v = options.get(key, default)
    if isinstance(v, bool):
        return default
    if isinstance(v, (int, float)):
        return float(v)
    return default


def _options_int(options: Any, key: str, default: int) -> int:
    if not isinstance(options, dict):
        return default
    v = options.get(key, default)
    if isinstance(v, bool):
        return default
    if isinstance(v, int):
        return v
    if isinstance(v, float):
        return int(v)
    return default


def _options_str(options: Any, key: str, default: str) -> str:
    if not isinstance(options, dict):
        return default
    v = options.get(key, default)
    if isinstance(v, str):
        s = v.strip()
        return s if s else default
    return default


def _options_choice(
    options: Any,
    key: str,
    *,
    default: str,
    allowed: tuple[str, ...],
) -> str:
    v = _options_str(options, key, default).lower()
    return v if v in allowed else default


def _debug_images_root() -> Path:
    base = (os.environ.get("DOCCONVERT_DATA_DIR") or "").strip()
    if base:
        return Path(base) / "debug" / "docling_images"
    return Path.cwd() / ".docconvert-debug" / "docling_images"


def _ocr_tweak_options(raw_opts: Any) -> dict[str, Any]:
    """占位符 OCR 小图放大 / 重试参数（缓解极小裁剪导致 rapidocr_no_boxes）。"""
    return {
        "small_side_threshold": max(8, _options_int(raw_opts, "docling_ocr_small_side_threshold", 96)),
        "target_min_side": max(32, _options_int(raw_opts, "docling_ocr_target_min_side", 256)),
        "max_long_edge": max(256, _options_int(raw_opts, "docling_ocr_max_upscaled_long_edge", 4096)),
        "retry_scale": max(1.0, _options_float(raw_opts, "docling_ocr_retry_scale", 2.0)),
        "retry_on_fail": _options_flag(raw_opts, "docling_ocr_retry_on_fail", True),
    }


def _upscale_pil_min_side(
    pil: Any,
    *,
    target_min_side: int,
    max_long_edge: int,
) -> tuple[Any, float]:
    """将 PIL 放大至最短边至少为 target_min_side（不超过 max_long_edge 长边）。"""
    from PIL import Image as PILImage

    w, h = pil.size
    m = min(w, h)
    if m <= 0:
        return pil, 1.0
    if m >= target_min_side:
        return pil, 1.0
    scale = target_min_side / float(m)
    nw = max(1, int(round(w * scale)))
    nh = max(1, int(round(h * scale)))
    long = max(nw, nh)
    if long > max_long_edge:
        r = max_long_edge / float(long)
        nw = max(1, int(round(nw * r)))
        nh = max(1, int(round(nh * r)))
        scale = scale * r
    return pil.resize((nw, nh), PILImage.Resampling.LANCZOS), scale


def _scale_pil_by_factor(
    pil: Any,
    factor: float,
    max_long_edge: int,
) -> tuple[Any, float]:
    """按比例放大，长边不超过 max_long_edge。"""
    from PIL import Image as PILImage

    if factor <= 1.0:
        return pil, 1.0
    w, h = pil.size
    nw = max(1, int(round(w * factor)))
    nh = max(1, int(round(h * factor)))
    long = max(nw, nh)
    if long > max_long_edge:
        r = max_long_edge / float(long)
        nw = max(1, int(round(nw * r)))
        nh = max(1, int(round(nh * r)))
        factor = factor * r
    return pil.resize((nw, nh), PILImage.Resampling.LANCZOS), factor


def _lines_from_rapidocr_out(out: Any) -> tuple[list[str] | None, str]:
    """解析 RapidOCR 输出；失败时返回 (None, reason_code)。"""
    if out is None:
        return None, "rapidocr_returned_none"
    boxes = getattr(out, "boxes", None)
    txts = getattr(out, "txts", None)
    if boxes is None:
        return None, "rapidocr_no_boxes"
    try:
        if hasattr(boxes, "__len__") and len(boxes) == 0:
            return None, "rapidocr_no_boxes"
    except Exception:
        pass
    if not txts:
        return None, "rapidocr_txts_empty"
    lines = [t.strip() for t in txts if t and str(t).strip()]
    if not lines:
        return None, "all_text_lines_blank"
    return lines, "ok"


def _build_document_converter(raw_opts: Any, log: logging.Logger) -> Any:
    """构造 DocumentConverter：默认开启页渲染图，否则 PictureItem.get_image 会因 page.image 为空而返回 None。"""
    from docling.datamodel.base_models import InputFormat
    from docling.document_converter import DocumentConverter, ImageFormatOption, PdfFormatOption
    from docling.datamodel.pipeline_options import ThreadedPdfPipelineOptions

    gen_page = _options_flag(raw_opts, "docling_generate_page_images", True)
    gen_pic = _options_flag(raw_opts, "docling_generate_picture_images", True)
    scale = _options_float(raw_opts, "docling_images_scale", 1.0)
    if scale <= 0:
        scale = 1.0

    pdf_opts = ThreadedPdfPipelineOptions(
        generate_page_images=gen_page,
        generate_picture_images=gen_pic,
        images_scale=scale,
    )
    format_options = {
        InputFormat.PDF: PdfFormatOption(pipeline_options=pdf_opts),
        InputFormat.IMAGE: ImageFormatOption(pipeline_options=pdf_opts),
    }
    log.info(
        "docling pipeline: generate_page_images=%s generate_picture_images=%s images_scale=%s",
        gen_page,
        gen_pic,
        scale,
    )
    return DocumentConverter(format_options=format_options)


def _collect_picture_items(doc: "DoclingDocument") -> list:
    from docling_core.types.doc import PictureItem

    out: list = []
    for item, _level in doc.iterate_items(traverse_pictures=False):
        if isinstance(item, PictureItem):
            out.append(item)
    return out


def _collect_page_images(doc: "DoclingDocument", log: logging.Logger) -> list[Any]:
    """收集每页整页渲染图（按页序）。"""
    pages_obj = getattr(doc, "pages", None)
    if pages_obj is None:
        return []
    if isinstance(pages_obj, dict):
        page_entries = sorted(pages_obj.items(), key=lambda kv: kv[0])
        page_objs = [v for _k, v in page_entries]
    elif isinstance(pages_obj, list):
        page_objs = pages_obj
    else:
        return []

    out: list[Any] = []
    for i, page in enumerate(page_objs):
        pil = None
        img = getattr(page, "image", None)
        if img is not None:
            if hasattr(img, "pil_image"):
                pil = getattr(img, "pil_image", None)
            elif hasattr(img, "to_pil"):
                try:
                    pil = img.to_pil()
                except Exception:
                    pil = None
        if pil is None and hasattr(page, "pil_image"):
            pil = getattr(page, "pil_image", None)
        if pil is None:
            log.warning("image_placeholder_ocr page_image page=%d reason=pil_none", i)
        out.append(pil)
    return out


def _ocr_pil_with_rapidocr(
    ocr_engine: Any,
    pil: Any,
    *,
    log: logging.Logger,
    slot: int,
    verbose: bool,
    ocr_tweak: dict[str, Any],
) -> str:
    """对单张图跑 RapidOCR；极小裁剪会先放大，失败时可再放大重试一次。"""
    if pil is None:
        log.warning(
            "image_placeholder_ocr slot=%d outcome=empty reason=pil_none "
            "(PictureItem.get_image 未返回图像)",
            slot,
        )
        return ""
    import numpy as np
    from PIL import Image as PILImage

    if not isinstance(pil, PILImage.Image):
        log.warning(
            "image_placeholder_ocr slot=%d outcome=empty reason=pil_wrong_type type=%s",
            slot,
            type(pil).__name__,
        )
        return ""

    orig_w, orig_h = pil.size
    if pil.mode != "RGB":
        pil = pil.convert("RGB")

    thr = int(ocr_tweak["small_side_threshold"])
    target_min = int(ocr_tweak["target_min_side"])
    max_long = int(ocr_tweak["max_long_edge"])
    retry_scale = float(ocr_tweak["retry_scale"])
    do_retry = bool(ocr_tweak["retry_on_fail"])

    pil_work, pre_scale = pil, 1.0
    if min(orig_w, orig_h) < thr:
        pil_work, pre_scale = _upscale_pil_min_side(
            pil_work, target_min_side=target_min, max_long_edge=max_long
        )
        if pre_scale > 1.01 and verbose:
            tw, th = pil_work.size
            log.info(
                "image_placeholder_ocr slot=%d small_crop pre_upscale orig=%dx%d -> %dx%d factor=%.3f",
                slot,
                orig_w,
                orig_h,
                tw,
                th,
                pre_scale,
            )

    def _run_once(p_img: Any, phase: str) -> tuple[list[str] | None, str, Any]:
        arr = np.asarray(p_img)
        try:
            out = ocr_engine(arr)
        except Exception as exc:
            log.warning(
                "image_placeholder_ocr slot=%d outcome=empty reason=rapidocr_exception phase=%s err=%s",
                slot,
                phase,
                exc,
                exc_info=verbose,
            )
            return None, "rapidocr_exception", None
        lines, reason = _lines_from_rapidocr_out(out)
        return lines, reason, out

    lines, reason, last_out = _run_once(pil_work, "primary")
    if reason == "rapidocr_exception":
        return ""
    if lines:
        if verbose and last_out is not None:
            scores = getattr(last_out, "scores", None)
            _bx = getattr(last_out, "boxes", None)
            n_boxes = len(_bx) if _bx is not None and hasattr(_bx, "__len__") else 0
            score_hint = ""
            if scores is not None and hasattr(scores, "__len__") and len(scores) > 0:
                try:
                    score_hint = f" score_min={float(min(scores)):.4f}"
                except (TypeError, ValueError):
                    pass
            log.info(
                "image_placeholder_ocr slot=%d outcome=ok size=%dx%d lines=%d boxes=%d%s",
                slot,
                orig_w,
                orig_h,
                len(lines),
                n_boxes,
                score_hint,
            )
        return "\n".join(lines)

    # 首轮失败：对「小原图」且检测/文本失败时再放大重试一次
    if do_retry and min(orig_w, orig_h) < thr and reason in (
        "rapidocr_no_boxes",
        "rapidocr_txts_empty",
        "all_text_lines_blank",
    ):
        pil_retry, rfac = _scale_pil_by_factor(pil_work, retry_scale, max_long)
        if rfac > 1.01:
            lines2, reason2, last_out2 = _run_once(pil_retry, "retry_upscale")
            if reason2 == "rapidocr_exception":
                return ""
            if lines2:
                tw, th = pil_retry.size
                if verbose:
                    log.info(
                        "image_placeholder_ocr slot=%d outcome=ok after_retry_upscale "
                        "orig=%dx%d work=%dx%d retry_factor=%.3f first_fail=%s lines=%d",
                        slot,
                        orig_w,
                        orig_h,
                        tw,
                        th,
                        rfac,
                        reason,
                        len(lines2),
                    )
                return "\n".join(lines2)
            if verbose:
                log.warning(
                    "image_placeholder_ocr slot=%d retry_upscale still_empty "
                    "orig=%dx%d reason=%s retry_reason=%s",
                    slot,
                    orig_w,
                    orig_h,
                    reason,
                    reason2,
                )

    # 记录与旧版一致的失败语义
    if reason == "rapidocr_returned_none":
        log.warning(
            "image_placeholder_ocr slot=%d outcome=empty reason=rapidocr_returned_none size=%dx%d",
            slot,
            orig_w,
            orig_h,
        )
    elif reason == "rapidocr_no_boxes":
        log.warning(
            "image_placeholder_ocr slot=%d outcome=empty reason=rapidocr_no_boxes size=%dx%d",
            slot,
            orig_w,
            orig_h,
        )
    elif reason == "rapidocr_txts_empty":
        log.warning(
            "image_placeholder_ocr slot=%d outcome=empty reason=rapidocr_txts_empty size=%dx%d",
            slot,
            orig_w,
            orig_h,
        )
    elif reason == "all_text_lines_blank":
        log.warning(
            "image_placeholder_ocr slot=%d outcome=empty reason=all_text_lines_blank size=%dx%d",
            slot,
            orig_w,
            orig_h,
        )
    return ""


def _format_ocr_replacement(text: str, *, mode: str) -> str:
    """将占位符替换为带标注的 Markdown 块（避免与正文混淆）。"""
    t = _postprocess_ocr_text(text, mode=mode)
    if not t:
        return "\n\n*[图片 OCR：未识别到文字]*\n\n"
    lines = t.split("\n")
    quoted = "\n".join(f"> {line}" for line in lines)
    return f"\n\n**〔图片 OCR〕**\n{quoted}\n\n"


def _format_page_ocr_replacement(page_no: int, text: str, *, mode: str) -> str:
    """整页 OCR 结果块，带页号。"""
    t = _postprocess_ocr_text(text, mode=mode)
    if not t:
        return f"\n\n**〔整页 OCR 第{page_no}页〕**\n\n*[未识别到文字]*\n\n"
    lines = t.split("\n")
    quoted = "\n".join(f"> {line}" for line in lines)
    return f"\n\n**〔整页 OCR 第{page_no}页〕**\n{quoted}\n\n"


def _postprocess_ocr_text(text: str, *, mode: str) -> str:
    """轻量 OCR 文本后处理：去噪、合并逐字断行、拼接常见日期表达。"""
    mode_norm = mode.strip().lower() if isinstance(mode, str) else "strict"
    if mode_norm not in ("strict", "lenient"):
        mode_norm = "strict"

    raw_lines = [ln.strip() for ln in text.splitlines() if ln and ln.strip()]
    if not raw_lines:
        return ""

    # strict: 过滤孤立英文字母/符号噪声；lenient: 仅过滤纯符号，保留单字母/单字结果。
    if mode_norm == "strict":
        lines = [
            ln
            for ln in raw_lines
            if not re.fullmatch(r"[A-Za-z]|[^\w\u4e00-\u9fff]+", ln)
        ]
    else:
        lines = [ln for ln in raw_lines if not re.fullmatch(r"[^\w\u4e00-\u9fff]+", ln)]
    if not lines:
        return ""

    short_ratio = sum(1 for ln in lines if len(ln) <= 2) / len(lines)
    # 大量短行时通常是逐字断行（如“2021/年/11/月/30/日”），合并为一行可读文本。
    if len(lines) >= 4 and short_ratio >= 0.7:
        merged = "".join(lines)
        merged = re.sub(r"\s+", "", merged)
        merged = re.sub(r"(\d{1,4})年(\d{1,2})月(\d{1,2})日", r"\1年\2月\3日", merged)
        return merged.strip()

    return "\n".join(lines).strip()


def _dump_extracted_images(
    *,
    doc: "DoclingDocument",
    pics: list,
    temp_dir: str,
    limit: int,
    min_side: int,
    log: logging.Logger,
) -> str | None:
    if not pics:
        return None
    root = _debug_images_root()
    job = Path(temp_dir).name.strip() or "unknown"
    out_dir = root / job
    out_dir.mkdir(parents=True, exist_ok=True)

    saved = 0
    cap = max(1, limit)
    min_side = max(1, int(min_side))
    slots_meta: list[dict[str, Any]] = []

    def _float_or_none(v: Any) -> float | None:
        try:
            return float(v)
        except (TypeError, ValueError):
            return None

    def _extract_page_and_bbox(pic: Any) -> tuple[int | None, list[float] | None]:
        page_no: int | None = None
        bbox: list[float] | None = None

        # 常见字段：prov / provenance（可能是列表）
        prov = getattr(pic, "prov", None)
        if prov is None:
            prov = getattr(pic, "provenance", None)
        if isinstance(prov, list) and prov:
            p0 = prov[0]
            if page_no is None:
                pn = getattr(p0, "page_no", None)
                if pn is None:
                    pn = getattr(p0, "page", None)
                try:
                    page_no = int(pn) if pn is not None else None
                except (TypeError, ValueError):
                    page_no = None
            bb = getattr(p0, "bbox", None)
            if bb is not None:
                if isinstance(bb, (list, tuple)) and len(bb) == 4:
                    vals = [_float_or_none(x) for x in bb]
                    if all(v is not None for v in vals):
                        bbox = [float(v) for v in vals]  # type: ignore[arg-type]
                else:
                    vals = [
                        _float_or_none(getattr(bb, "l", None)),
                        _float_or_none(getattr(bb, "t", None)),
                        _float_or_none(getattr(bb, "r", None)),
                        _float_or_none(getattr(bb, "b", None)),
                    ]
                    if all(v is not None for v in vals):
                        bbox = [float(v) for v in vals]  # type: ignore[arg-type]
        return page_no, bbox

    for i, pic in enumerate(pics):
        if saved >= cap:
            break
        slot_name = f"slot_{i:03d}.png"
        page_no, bbox = _extract_page_and_bbox(pic)
        slot_info: dict[str, Any] = {
            "slot": i,
            "filename": slot_name,
            "page_no": page_no,
            "bbox": bbox,
            "saved": False,
            "reason": "",
        }
        try:
            pil = pic.get_image(doc)
        except Exception as exc:
            log.warning("dump_extracted_images slot=%d get_image_failed err=%s", i, exc)
            slot_info["reason"] = f"get_image_failed: {exc}"
            slots_meta.append(slot_info)
            continue
        if pil is None:
            slot_info["reason"] = "pil_none"
            slots_meta.append(slot_info)
            continue
        if pil.mode != "RGB":
            pil = pil.convert("RGB")
        w, h = pil.size
        slot_info["width"] = int(w)
        slot_info["height"] = int(h)
        if min(w, h) < min_side:
            slot_info["reason"] = f"filtered_small_side<{min_side}"
            slots_meta.append(slot_info)
            continue
        target = out_dir / slot_name
        try:
            pil.save(target, format="PNG")
            saved += 1
            slot_info["saved"] = True
            slot_info["reason"] = "ok"
            slots_meta.append(slot_info)
        except Exception as exc:
            log.warning("dump_extracted_images slot=%d save_failed path=%s err=%s", i, target, exc)
            slot_info["reason"] = f"save_failed: {exc}"
            slots_meta.append(slot_info)
    if slots_meta:
        try:
            meta_path = out_dir / "slots.json"
            meta_path.write_text(
                json.dumps(
                    {
                        "total_picture_items": len(pics),
                        "saved_count": saved,
                        "limit": cap,
                        "min_side": min_side,
                        "slots": slots_meta,
                    },
                    ensure_ascii=False,
                    indent=2,
                ),
                encoding="utf-8",
            )
        except Exception as exc:
            log.warning("dump_extracted_images write slots.json failed err=%s", exc)
    if saved > 0:
        log.info(
            "dump_extracted_images saved=%d total_pictures=%d min_side=%d dir=%s",
            saved,
            len(pics),
            min_side,
            out_dir,
        )
        return str(out_dir)
    return None


def _dump_page_images(
    *,
    page_images: list[Any],
    temp_dir: str,
    limit: int,
    min_side: int,
    log: logging.Logger,
) -> str | None:
    if not page_images:
        return None
    root = _debug_images_root()
    job = Path(temp_dir).name.strip() or "unknown"
    out_dir = root / job
    out_dir.mkdir(parents=True, exist_ok=True)

    saved = 0
    cap = max(1, limit)
    min_side = max(1, int(min_side))
    pages_meta: list[dict[str, Any]] = []

    for i, pil in enumerate(page_images):
        if i >= cap:
            break
        name = f"page_{i + 1:03d}.png"
        info: dict[str, Any] = {
            "page_no": i + 1,
            "filename": name,
            "saved": False,
            "reason": "",
        }
        if pil is None:
            info["reason"] = "pil_none"
            pages_meta.append(info)
            continue
        if pil.mode != "RGB":
            pil = pil.convert("RGB")
        w, h = pil.size
        info["width"] = int(w)
        info["height"] = int(h)
        if min(w, h) < min_side:
            info["reason"] = f"filtered_small_side<{min_side}"
            pages_meta.append(info)
            continue
        target = out_dir / name
        try:
            pil.save(target, format="PNG")
            saved += 1
            info["saved"] = True
            info["reason"] = "ok"
        except Exception as exc:
            log.warning("dump_page_images page=%d save_failed path=%s err=%s", i + 1, target, exc)
            info["reason"] = f"save_failed: {exc}"
        pages_meta.append(info)

    if pages_meta:
        try:
            meta_path = out_dir / "pages.json"
            meta_path.write_text(
                json.dumps(
                    {
                        "total_pages": len(page_images),
                        "saved_count": saved,
                        "limit": cap,
                        "min_side": min_side,
                        "pages": pages_meta,
                    },
                    ensure_ascii=False,
                    indent=2,
                ),
                encoding="utf-8",
            )
        except Exception as exc:
            log.warning("dump_page_images write pages.json failed err=%s", exc)
    if saved > 0:
        log.info(
            "dump_page_images saved=%d total_pages=%d min_side=%d dir=%s",
            saved,
            len(page_images),
            min_side,
            out_dir,
        )
        return str(out_dir)
    return None


def _inject_ocr_after_image_placeholders(
    doc: "DoclingDocument",
    markdown: str,
    placeholder: str,
    *,
    log: logging.Logger,
    ocr_log: bool,
    ocr_tweak: dict[str, Any],
    ocr_post_mode: str,
    ocr_image_source: str,
) -> str:
    if placeholder not in markdown and ocr_image_source != "page_image":
        return markdown
    parts = markdown.split(placeholder) if placeholder in markdown else [markdown]
    if len(parts) <= 1 and ocr_image_source != "page_image":
        return markdown
    placeholder_count = len(parts) - 1
    pics = _collect_picture_items(doc)
    page_images = _collect_page_images(doc, log) if ocr_image_source == "page_image" else []
    source_name = "page_image" if ocr_image_source == "page_image" else "picture_item"
    source_count = len(page_images) if ocr_image_source == "page_image" else len(pics)
    n = source_count if ocr_image_source == "page_image" else placeholder_count
    if ocr_log:
        log.info(
            "image_placeholder_ocr: placeholders=%d slots=%d image_source=%s source_count=%d picture_items=%d placeholder=%r",
            placeholder_count,
            n,
            source_name,
            source_count,
            len(pics),
            placeholder,
        )
    if n > source_count:
        log.warning(
            "image_placeholder_ocr: placeholders 多于可用图像，多出的槽位将没有图像 "
            "(slots %d..%d)",
            source_count,
            n - 1,
        )
    ocr_engine = None
    nonempty_count = 0
    empty_count = 0

    def _engine():
        nonlocal ocr_engine
        if ocr_engine is None:
            from rapidocr import RapidOCR

            _quiet_rapidocr_logging()
            ro_params = _rapidocr_params_for_docconvert()
            ocr_engine = RapidOCR(params=ro_params) if ro_params else RapidOCR()
            _quiet_rapidocr_logging()
            if ocr_log:
                try:
                    import rapidocr as _ro

                    ver = getattr(_ro, "__version__", "unknown")
                except Exception:
                    ver = "unknown"
                log.info("image_placeholder_ocr: RapidOCR engine ready rapidocr=%s", ver)
        return ocr_engine

    out: list[str] = [parts[0]]
    for i in range(n):
        if ocr_image_source == "page_image":
            pil = page_images[i] if i < len(page_images) else None
            if pil is None:
                log.warning(
                    "image_placeholder_ocr slot=%d outcome=empty reason=no_page_image "
                    "(占位符索引超出页面数量或页面图为空)",
                    i,
                )
        else:
            pic = pics[i] if i < len(pics) else None
            if pic is None:
                log.warning(
                    "image_placeholder_ocr slot=%d outcome=empty reason=no_picture_item "
                    "(占位符索引超出 PictureItem 数量)",
                    i,
                )
            pil = pic.get_image(doc) if pic is not None else None
        eng = _engine()
        txt = _ocr_pil_with_rapidocr(
            eng, pil, log=log, slot=i, verbose=ocr_log, ocr_tweak=ocr_tweak
        )
        if txt.strip():
            nonempty_count += 1
        else:
            empty_count += 1
        if ocr_image_source == "page_image":
            if i < placeholder_count:
                out.append(_format_ocr_replacement(txt, mode=ocr_post_mode))
                out.append(parts[i + 1])
            else:
                out.append(_format_page_ocr_replacement(i + 1, txt, mode=ocr_post_mode))
        else:
            out.append(_format_ocr_replacement(txt, mode=ocr_post_mode))
            out.append(parts[i + 1])
    if ocr_image_source == "page_image" and placeholder_count == 0 and n > 0:
        out.append("\n\n")
    # 汇总行放在循环末尾，便于在截断时仍尽量靠近 stderr 尾部（配合 RapidOCR 降噪与 Core 提高 docling 上限）
    log.info(
        "image_placeholder_ocr summary: slots=%d placeholders=%d nonempty=%d empty=%d source=%s",
        n,
        placeholder_count,
        nonempty_count,
        empty_count,
        source_name,
    )
    _flush_diagnostic_streams(log)
    return "".join(out)


def run(ctx: dict) -> dict:
    """
    JSON-RPC 入口。ctx 字段：
      input_path, output_path, in_format, out_format, temp_dir
      options（可选）:
        docling_image_placeholder_ocr: bool  默认 True；为 False 时不做占位符 OCR
        docling_image_placeholder_ocr_log: bool  默认 True；为 False 时不向 stderr 输出 OCR 诊断（仅保留严重告警）
        docling_ocr_image_source: str  picture_item|page_image，默认 picture_item；占位符 OCR 时取图来源
        docling_ocr_postprocess_mode: str  strict|lenient，默认 strict；strict 会过滤孤立单字母噪声，lenient 更宽松保留疑似文本
        docling_dump_extracted_images: bool 默认 False；为 True 时将 Docling 提取到的图片落盘，便于人工核对
        docling_dump_extracted_images_limit: int 默认 20；最多保存多少张提取图片/页图
        docling_dump_extracted_images_min_side: int 默认 80；仅保存最短边不小于该阈值的图片（过滤角标/徽标等小图）
        docling_generate_page_images: bool  默认 True；占位符 OCR 需页渲染图，否则 PictureItem.get_image 常为 None
        docling_generate_picture_images: bool  默认 True；提取 PDF 内嵌位图，供 FloatingItem.image 路径使用
        docling_images_scale: number  默认 1.0；页图缩放，略增可提升 OCR 清晰度（更慢、更占内存）
        image_placeholder: str  与 Docling 导出占位符一致，默认 `<!-- image -->`
        docling_ocr_small_side_threshold: int  默认 96；原图最短边低于此值视为「小裁剪」，先放大再 OCR
        docling_ocr_target_min_side: int  默认 256；小图首次放大时目标最短边
        docling_ocr_max_upscaled_long_edge: int  默认 4096；放大后长边上限
        docling_ocr_retry_scale: number  默认 2.0；首轮失败时对已放大工作图再乘以此比例重试
        docling_ocr_retry_on_fail: bool  默认 True；小图在 no_boxes/空文本 等失败时是否再放大重试
    """
    input_path = Path(ctx["input_path"])
    output_path = Path(ctx["output_path"])
    out_format = ctx.get("out_format", "markdown")
    input_sz = input_path.stat().st_size if input_path.is_file() else 0
    raw_opts = ctx.get("options")
    ocr_enabled = _options_flag(raw_opts, "docling_image_placeholder_ocr", True)
    ocr_log = _options_flag(raw_opts, "docling_image_placeholder_ocr_log", True)
    ocr_image_source = _options_choice(
        raw_opts,
        "docling_ocr_image_source",
        default="picture_item",
        allowed=("picture_item", "page_image"),
    )
    ocr_post_mode = _options_str(
        raw_opts, "docling_ocr_postprocess_mode", "strict"
    ).lower()
    if ocr_post_mode not in ("strict", "lenient"):
        ocr_post_mode = "strict"
    dump_images = _options_flag(raw_opts, "docling_dump_extracted_images", False)
    dump_images_limit = max(
        1, _options_int(raw_opts, "docling_dump_extracted_images_limit", 20)
    )
    dump_images_min_side = max(
        1, _options_int(raw_opts, "docling_dump_extracted_images_min_side", 80)
    )
    placeholder = DEFAULT_IMAGE_PLACEHOLDER
    if isinstance(raw_opts, dict):
        ph = raw_opts.get("image_placeholder")
        if isinstance(ph, str) and ph.strip():
            placeholder = ph.strip()

    if out_format not in ("markdown", "json"):
        raise ValueError(f"docling_adapter: unsupported output format '{out_format}'")

    try:
        from docling.document_converter import DocumentConverter
    except ImportError as e:
        raise RuntimeError(
            "未安装 Docling 或当前 Python 环境不满足要求。"
            "请在 Python≥3.10 的 venv 中执行：pip install docling（包体较大，是否随包打包请见 README）；"
            "并设置 DOCCONVERT_PYTHON 指向该解释器，或重建 python/.venv 后重新 npm run bundle-python 再打包。"
        ) from e

    log = _plugin_logger()
    log.info(
        "convert start input=%s size_bytes=%d out_format=%s ocr_enabled=%s ocr_log=%s ocr_engine=rapidocr ocr_image_source=%s ocr_post_mode=%s",
        input_path,
        input_sz,
        out_format,
        ocr_enabled,
        ocr_log,
        ocr_image_source,
        ocr_post_mode,
    )

    converter = _build_document_converter(raw_opts, log)
    result = converter.convert(str(input_path))
    doc = result.document

    output_path.parent.mkdir(parents=True, exist_ok=True)

    if out_format == "markdown":
        content = doc.export_to_markdown()
        if not content and input_sz > 0:
            raise ValueError(
                "Docling 导出的 Markdown 为空，但输入文件非空。常见于扫描版 PDF 未启用版面解析、"
                "或需换用其他插件；请查看 Core 日志。"
            )
        pics = _collect_picture_items(doc)
        page_images = _collect_page_images(doc, log) if ocr_image_source == "page_image" else []
        dumped_dir: str | None = None
        if dump_images:
            if ocr_image_source == "page_image":
                dumped_dir = _dump_page_images(
                    page_images=page_images,
                    temp_dir=str(ctx.get("temp_dir", "")),
                    limit=dump_images_limit,
                    min_side=dump_images_min_side,
                    log=log,
                )
            else:
                dumped_dir = _dump_extracted_images(
                    doc=doc,
                    pics=pics,
                    temp_dir=str(ctx.get("temp_dir", "")),
                    limit=dump_images_limit,
                    min_side=dump_images_min_side,
                    log=log,
                )
        if ocr_enabled and (placeholder in content or ocr_image_source == "page_image"):
            try:
                ocr_tweak = _ocr_tweak_options(raw_opts)
                content = _inject_ocr_after_image_placeholders(
                    doc,
                    content,
                    placeholder,
                    log=log,
                    ocr_log=ocr_log,
                    ocr_tweak=ocr_tweak,
                    ocr_post_mode=ocr_post_mode,
                    ocr_image_source=ocr_image_source,
                )
            except Exception as exc:
                # OCR 失败不应整任务失败：保留原文并附加说明
                content = (
                    content
                    + "\n\n<!-- docling_adapter: 图片占位符 OCR 失败: "
                    + str(exc).replace("--", "—")
                    + " -->\n"
                )
        log.info(
            "convert done output=%s char_count=%d",
            output_path,
            len(content),
        )
        _flush_diagnostic_streams(log)
        output_path.write_text(content, encoding="utf-8")
        return {
            "status": "ok",
            "output_path": str(output_path),
            "char_count": len(content),
            "debug_images_dir": dumped_dir,
        }
    elif out_format == "json":
        content = json.dumps(doc.export_to_dict(), ensure_ascii=False, indent=2)
        log.info(
            "convert done output=%s char_count=%d",
            output_path,
            len(content),
        )
        _flush_diagnostic_streams(log)
        output_path.write_text(content, encoding="utf-8")
        return {
            "status": "ok",
            "output_path": str(output_path),
            "char_count": len(content),
        }
    raise RuntimeError("unreachable")


if __name__ == "__main__":
    req = json.loads(sys.stdin.read())
    try:
        result = run(req["params"])
        print(json.dumps({"jsonrpc": "2.0", "id": req.get("id", 1), "result": result}))
    except Exception as exc:
        print(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": req.get("id", 1),
                    "error": {"code": -32000, "message": str(exc)},
                }
            )
        )
